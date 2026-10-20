//! Project keys, encryption of what the coordinator relays, and opaque slice IDs.
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use miliastra_core::{
    opaque::{self, Envelope},
    *,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use zeroize::{Zeroize, Zeroizing};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid private metadata"
    );
    Ok(Zeroizing::new(
        (0..s.len())
            .step_by(2)
            .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
            .collect::<Result<_>>()?,
    ))
}
fn random_key() -> String {
    let mut key = Aes256Gcm::generate_key(OsRng);
    let result = hex(&key);
    key.as_mut_slice().zeroize();
    result
}
fn key_id(key: &str) -> Result<String> {
    let bytes = unhex(key)?;
    ensure!(bytes.len() == 32, "Invalid project key");
    Ok(miliastra_core::hash(&bytes))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    current: String,
    index: String,
    keys: BTreeMap<String, String>,
}
impl Drop for Keys {
    fn drop(&mut self) {
        self.index.zeroize();
        for key in self.keys.values_mut() {
            key.zeroize();
        }
    }
}
impl Keys {
    pub(crate) fn new() -> Self {
        let key = random_key();
        let current = key_id(&key).unwrap();
        Self {
            current: current.clone(),
            index: random_key(),
            keys: BTreeMap::from([(current, key)]),
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        key_id(&self.index)?;
        ensure!(
            !self.keys.is_empty()
                && self.keys.len() <= 128
                && self.keys.contains_key(&self.current),
            "Invalid Encryption Key bundle"
        );
        for (id, key) in &self.keys {
            ensure!(key_id(key)? == *id, "Invalid historical key");
        }
        Ok(())
    }
    pub(crate) fn export(&self) -> Result<String> {
        self.validate()?;
        let clear = Zeroizing::new(serde_json::to_vec(self)?);
        Ok(format!("mcollab-key-v2:{}", URL_SAFE_NO_PAD.encode(&clear)))
    }
    pub(crate) fn import(value: &str) -> Result<Self> {
        ensure!(value.len() <= 40000, "Encryption Key bundle is too large");
        let encoded = value
            .strip_prefix("mcollab-key-v2:")
            .context("Complete Encryption Key required")?;
        let clear = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded)?);
        let keys: Self = serde_json::from_slice(&clear)?;
        keys.validate()?;
        Ok(keys)
    }
    /// Only tests rotate keys; the app can't.
    #[cfg(test)]
    pub(crate) fn rotate(&mut self) -> Result<()> {
        ensure!(self.keys.len() < 128, "Too many key generations");
        let key = random_key();
        self.current = key_id(&key)?;
        self.keys.insert(self.current.clone(), key);
        Ok(())
    }
    pub(crate) fn commitment(
        &self,
        endpoint: &str,
        project: &str,
        purpose: &str,
        clear: &[u8],
    ) -> Result<String> {
        let normalized = crate::runtime::endpoint(endpoint)?;
        let endpoint = normalized.as_str();
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&unhex(&self.index)?)?;
        mac.update(&serde_json::to_vec(&(
            "miliastra-metadata-v2",
            endpoint,
            project,
            purpose,
            clear,
        ))?);
        Ok(hex(&mac.finalize().into_bytes()))
    }
    pub(crate) fn seal<T: Serialize + ?Sized>(
        &self,
        endpoint: &str,
        project: &str,
        purpose: &str,
        value: &T,
    ) -> Result<String> {
        let normalized = crate::runtime::endpoint(endpoint)?;
        let endpoint = normalized.as_str();
        let clear = Zeroizing::new(serde_json::to_vec(value)?);
        ensure!(
            clear.len() <= 32764 && purpose.len() <= 256,
            "Metadata is too large"
        );
        let size = (clear.len() + 4).next_power_of_two().max(256);
        let mut padded = Zeroizing::new(vec![0u8; size]);
        padded[..4].copy_from_slice(&(clear.len() as u32).to_be_bytes());
        padded[4..4 + clear.len()].copy_from_slice(&clear);
        let bytes = unhex(
            self.keys
                .get(&self.current)
                .context("Encryption Key unavailable")?,
        )?;
        let cipher =
            Aes256Gcm::new_from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid key"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut e = Envelope {
            v: 2,
            k: self.current.clone(),
            p: purpose.into(),
            c: self.commitment(endpoint, project, purpose, &clear)?,
            n: hex(&nonce),
            d: String::new(),
        };
        let aad =
            serde_json::to_vec(&("miliastra-metadata-v2", endpoint, project, &e.p, &e.k, &e.c))?;
        e.d = hex(&cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &padded,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("Encryption failed"))?);
        Ok(format!("m2:{}", serde_json::to_string(&e)?))
    }
    pub(crate) fn open<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        project: &str,
        purpose: &str,
        sealed: &str,
    ) -> Result<T> {
        let normalized = crate::runtime::endpoint(endpoint)?;
        let endpoint = normalized.as_str();
        opaque::field(sealed, purpose)?;
        let e = opaque::parse(sealed)?;
        let key = unhex(
            self.keys
                .get(&e.k)
                .context("Historical Encryption Key required")?,
        )?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("Invalid key"))?;
        let nonce = unhex(&e.n)?;
        let ciphertext = unhex(&e.d)?;
        let aad =
            serde_json::to_vec(&("miliastra-metadata-v2", endpoint, project, &e.p, &e.k, &e.c))?;
        let padded = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("Metadata authentication failed"))?,
        );
        ensure!(padded.len() >= 256, "Invalid metadata padding");
        let len = u32::from_be_bytes(padded[..4].try_into()?) as usize;
        ensure!(
            len <= padded.len() - 4 && padded[4 + len..].iter().all(|b| *b == 0),
            "Invalid metadata padding"
        );
        let clear = &padded[4..4 + len];
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&unhex(&self.index)?)?;
        mac.update(&serde_json::to_vec(&(
            "miliastra-metadata-v2",
            endpoint,
            project,
            purpose,
            clear,
        ))?);
        mac.verify_slice(&unhex(&e.c)?)
            .map_err(|_| anyhow::anyhow!("Metadata commitment authentication failed"))?;
        Ok(serde_json::from_slice(clear)?)
    }
}
#[cfg(test)]
const ENDPOINT: &str = "https://coordination.example.test";
#[cfg(test)]
const PROJECT: &str = "cae8d763-f63a-4655-9497-fd66a8914534";
#[test]
fn all_descriptive_metadata_roundtrips_without_plaintext_in_serialized_payloads() {
    let keys = Keys::new();
    let values = vec![
        ("project-name", "Secret world"),
        ("creator:member-id", "Private creator"),
        ("slice:5", "Private section"),
        ("summary:update-id", "A private changelog summary"),
        ("details:update-id", "Private details"),
        ("storage", "private-drive/folder-reference"),
        (
            "revision:update-id:5:token",
            "private checksum, real file size and artifact reference",
        ),
        ("access:request-id", "private@example.com"),
    ];
    let mut wires = vec![];
    for (purpose, value) in &values {
        let encrypted = keys.seal(ENDPOINT, PROJECT, purpose, value).unwrap();
        assert_eq!(
            keys.open::<String>(ENDPOINT, PROJECT, purpose, &encrypted)
                .unwrap(),
            *value
        );
        for (_, private) in &values {
            assert!(!encrypted.contains(private));
        }
        wires.push(encrypted);
    }
    let persisted = serde_json::to_string(&wires).unwrap();
    for (_, private) in &values {
        assert!(!persisted.contains(private));
    }
    for private in keys.keys.values().chain(std::iter::once(&keys.index)) {
        assert!(!persisted.contains(private));
    }
}
#[test]
fn metadata_rejects_cross_project_field_endpoint_and_ciphertext_substitution() {
    let keys = Keys::new();
    let sealed = keys
        .seal(ENDPOINT, PROJECT, "project-name", &"Secret")
        .unwrap();
    assert!(keys
        .open::<String>("https://other.example", PROJECT, "project-name", &sealed)
        .is_err());
    assert!(keys
        .open::<String>(ENDPOINT, "other-project", "project-name", &sealed)
        .is_err());
    assert!(keys
        .open::<String>(ENDPOINT, PROJECT, "summary:update", &sealed)
        .is_err());
    assert!(Keys::new()
        .open::<String>(ENDPOINT, PROJECT, "project-name", &sealed)
        .is_err());
    let mut e = opaque::parse(&sealed).unwrap();
    let old = e.d[..2].to_owned();
    e.d.replace_range(..2, if old == "00" { "01" } else { "00" });
    assert!(keys
        .open::<String>(
            ENDPOINT,
            PROJECT,
            "project-name",
            &format!("m2:{}", serde_json::to_string(&e).unwrap())
        )
        .is_err());
    assert!(opaque::parse("plaintext project").is_err());
}
#[test]
fn rotation_preserves_history_and_slice_ids_without_granting_old_keys_new_content() {
    let mut keys = Keys::new();
    let before = Keys::import(&keys.export().unwrap()).unwrap();
    let old = keys
        .seal(ENDPOINT, PROJECT, "summary:old", &"Old history")
        .unwrap();
    let comparison = keys.resource(ENDPOINT, PROJECT, "5").unwrap();
    keys.rotate().unwrap();
    let new = keys
        .seal(ENDPOINT, PROJECT, "summary:new", &"New history")
        .unwrap();
    assert_eq!(
        keys.open::<String>(ENDPOINT, PROJECT, "summary:old", &old)
            .unwrap(),
        "Old history"
    );
    assert!(before
        .open::<String>(ENDPOINT, PROJECT, "summary:new", &new)
        .is_err());
    assert_eq!(keys.resource(ENDPOINT, PROJECT, "5").unwrap(), comparison);
    let recovered = Keys::import(&keys.export().unwrap()).unwrap();
    assert_eq!(
        recovered
            .open::<String>(ENDPOINT, PROJECT, "summary:old", &old)
            .unwrap(),
        "Old history"
    );
    assert_eq!(
        recovered
            .open::<String>(ENDPOINT, PROJECT, "summary:new", &new)
            .unwrap(),
        "New history"
    );
    assert_ne!(
        keys.resource(ENDPOINT, "other-project", "5").unwrap(),
        comparison
    );
    assert!(Keys::import("password").is_err());
}
#[test]
fn randomized_ciphertexts_preserve_idempotency_and_hide_small_length_differences() {
    let keys = Keys::new();
    let a = keys
        .seal(ENDPOINT, PROJECT, "summary:one", &"Boats updated")
        .unwrap();
    let b = keys
        .seal(ENDPOINT, PROJECT, "summary:one", &"Boats updated")
        .unwrap();
    assert_ne!(a, b);
    let commitment = |v: &str| opaque::parse(v).unwrap().c;
    assert_eq!(commitment(&a), commitment(&b));
    let c = keys
        .seal(ENDPOINT, PROJECT, "summary:one", &"Islands updated")
        .unwrap();
    assert_ne!(commitment(&a), commitment(&c));
    assert_eq!(
        opaque::parse(&a).unwrap().d.len(),
        opaque::parse(&c).unwrap().d.len()
    );
    let huge = "x".repeat(32765);
    assert!(keys.seal(ENDPOINT, PROJECT, "summary:one", &huge).is_err());
}

impl Keys {
    pub(crate) fn current_key(&self) -> Result<&str> {
        self.keys
            .get(&self.current)
            .map(String::as_str)
            .context("Project Encryption Key unavailable")
    }
    pub(crate) fn same_project(&self, other: &Self) -> bool {
        self.index == other.index
    }
    pub(crate) fn merge_history(&mut self, other: &Self) {
        if self.keys.len() < other.keys.len()
            && self.keys.keys().all(|id| other.keys.contains_key(id))
        {
            self.current = other.current.clone();
        }
        for (id, key) in &other.keys {
            self.keys.entry(id.clone()).or_insert_with(|| key.clone());
        }
    }
    pub(crate) fn resource(&self, ep: &str, project: &str, section: &str) -> Result<String> {
        ensure!(valid_section(section), "Invalid slice");
        if section == "shared" {
            return Ok("shared".into());
        }
        self.commitment(ep, project, "resource-token", section.as_bytes())
    }
    pub(crate) fn resource_name(&self, ep: &str, project: &str, alias: &str) -> Result<String> {
        if alias == "shared" {
            return Ok("shared".into());
        }
        for n in 0..64 {
            let id = n.to_string();
            if self.resource(ep, project, &id)? == alias {
                return Ok(id);
            }
        }
        anyhow::bail!("Unknown private slice")
    }
    /// Decrypts live events and presence statuses.
    pub(crate) fn open_events(
        &self,
        ep: &str,
        project: &str,
        value: serde_json::Value,
    ) -> Result<serde_json::Value> {
        use serde_json::Value;
        match value {
            Value::Array(items) => Ok(Value::Array(
                items
                    .into_iter()
                    .map(|v| self.open_events(ep, project, v))
                    .collect::<Result<_>>()?,
            )),
            Value::Object(mut object) => {
                if let Some(member) = object
                    .get("member")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    if object.contains_key("status") {
                        let activity = object["status"].as_str().and_then(|sealed| {
                            self.open::<PresenceActivity>(
                                ep,
                                project,
                                &miliastra_core::presence::status_purpose(&member),
                                sealed,
                            )
                            .ok()
                        });
                        object.insert("status".into(), serde_json::to_value(activity)?);
                    }
                }
                if let Some(section) = object
                    .get("section")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    object.insert(
                        "section".into(),
                        self.resource_name(ep, project, &section)?.into(),
                    );
                }
                Ok(Value::Object(object))
            }
            other => Ok(other),
        }
    }
}

#[test]
fn private_notifications_and_access_outcomes_require_local_decryption() {
    let keys = Keys::new();
    let id = PROJECT;
    let alias = keys.resource(ENDPOINT, id, "5").unwrap();
    let event = serde_json::json!({"kind":"finish-request","section":alias,"from":"requester-id","to":"target-member"});
    assert!(!event.to_string().contains("\"5\""));
    let clear = keys.open_events(ENDPOINT, id, event.clone()).unwrap();
    assert_eq!(clear["section"], "5");
    let mut tampered = event;
    tampered["section"] = "d".repeat(64).into();
    assert!(keys.open_events(ENDPOINT, id, tampered).is_err());
    let request = uuid::Uuid::new_v4().to_string();
    let outcome = keys
        .seal(
            ENDPOINT,
            &id,
            &format!("access-result:{request}"),
            &serde_json::json!({
        "status":"granted","message":"Folder shared"}),
        )
        .unwrap();
    assert!(!outcome.contains("granted") && !outcome.contains("Folder shared"));
    let clear: serde_json::Value = keys
        .open(ENDPOINT, &id, &format!("access-result:{request}"), &outcome)
        .unwrap();
    assert_eq!(clear["status"], "granted");
    assert!(Keys::new()
        .open::<serde_json::Value>(ENDPOINT, &id, &format!("access-result:{request}"), &outcome)
        .is_err());
    let mut newer = Keys::new();
    let mut older = Keys::import(&newer.export().unwrap()).unwrap();
    newer.rotate().unwrap();
    let current = newer.current.clone();
    older.merge_history(&newer);
    assert_eq!(older.current, current);
}

pub(crate) fn validate_history_route(route: &str) -> Result<()> {
    if route.starts_with("/history") {
        let parsed = url::Url::parse(&format!("http://localhost{route}"))?;
        ensure!(parsed.path() == "/history", "Invalid history route");
        for (key, value) in parsed.query_pairs() {
            ensure!(
                key == "before" && value.parse::<u64>().is_ok(),
                "History only accepts pagination cursors"
            );
        }
    }
    Ok(())
}
#[test]
fn history_route_guard_rejects_private_filters_before_network_transfer() {
    assert!(validate_history_route("/history").is_ok());
    assert!(validate_history_route("/history?before=51").is_ok());
    for route in [
        "/history?q=private",
        "/history?author=Alex",
        "/history?section=5",
        "/history?from=1",
        "/history?before=51&q=private",
        "/history?before=secret",
    ] {
        assert!(validate_history_route(route).is_err());
    }
}

#[test]
fn presence_status_is_encrypted_per_member_and_unreadable_status_is_dropped() {
    let keys = Keys::new();
    let purpose = miliastra_core::presence::status_purpose("m1");
    let sealed = keys
        .seal(
            ENDPOINT,
            PROJECT,
            &purpose,
            &serde_json::json!({"game": true}),
        )
        .unwrap();
    assert!(!sealed.contains("game"));
    let malformed = keys
        .seal(
            ENDPOINT,
            PROJECT,
            &miliastra_core::presence::status_purpose("m5"),
            &serde_json::json!({"game": "true"}),
        )
        .unwrap();
    let idle = keys
        .seal(
            ENDPOINT,
            PROJECT,
            &miliastra_core::presence::status_purpose("m6"),
            &PresenceActivity { game: false },
        )
        .unwrap();
    let view = serde_json::json!({"presence": [
        {"member": "m1", "status": sealed.clone()},
        // A status copied from m1 can't be passed off as m2's.
        {"member": "m2", "status": sealed},
        {"member": "m3"},
        {"member": "m4", "status": {"game": true}},
        {"member": "m5", "status": malformed},
        {"member": "m6", "status": idle}
    ]});
    let open = serde_json::json!({"presence": keys.open_events(ENDPOINT, PROJECT, view["presence"].clone()).unwrap()});
    assert_eq!(open["presence"][0]["status"]["game"], true);
    assert!(open["presence"][1]["status"].is_null());
    assert!(open["presence"][2].get("status").is_none());
    assert!(
        open["presence"][3]["status"].is_null(),
        "plaintext activity refused"
    );
    assert!(
        open["presence"][4]["status"].is_null(),
        "malformed activity dropped"
    );
    assert_eq!(open["presence"][5]["status"]["game"], false);
    let typed: Vec<Presence> = serde_json::from_value(open["presence"].clone()).unwrap();
    assert!(matches!(
        typed[0].status,
        Some(PresenceStatus::Activity(PresenceActivity { game: true }))
    ));
    assert!(typed[1].status.is_none());
}

#[test]
fn current_event_decoder_accepts_only_authenticated_typed_presence() {
    let keys = Keys::new();
    let member = "creator";
    let sealed = keys
        .seal(
            ENDPOINT,
            PROJECT,
            &miliastra_core::presence::status_purpose(member),
            &serde_json::json!({"game":true}),
        )
        .unwrap();
    let value=keys.open_events(ENDPOINT,PROJECT,serde_json::json!([{"member":member,"status":sealed},{"member":"plain","status":{"game":true}}])).unwrap();
    assert_eq!(value[0]["status"]["game"], true);
    assert!(value[1]["status"].is_null());
}
