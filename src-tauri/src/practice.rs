//! A throwaway practice project for the native test scripts. Test builds only.
//! It lives on the test coordinator that tests/coordinator.cjs starts on this PC.
use crate::runtime::Core;
use anyhow::{ensure, Context, Result};
use miliastra_core::{
    binary::{self, Stage},
    *,
};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
fn vi(mut n: u64) -> Vec<u8> {
    let mut out = vec![];
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
    out
}
fn scalar(n: u64, v: u64) -> Vec<u8> {
    [vi(n << 3), vi(v)].concat()
}
fn string(n: u64, s: &[u8]) -> Vec<u8> {
    [vi((n << 3) | 2), vi(s.len() as u64), s.to_vec()].concat()
}
fn frame(kind: u32, payload: Vec<u8>) -> Vec<u8> {
    let mut b = vec![];
    for n in [
        payload.len() as u32 + 20,
        1,
        806,
        kind,
        payload.len() as u32,
    ] {
        b.extend(n.to_be_bytes());
    }
    b.extend(payload);
    b.extend(1657u32.to_be_bytes());
    b
}
pub fn slice(section: u8, label: &str) -> Vec<u8> {
    let inner = [scalar(1, 42), string(90, label.as_bytes())].concat();
    frame(
        6,
        [
            if section == 0 {
                vec![]
            } else {
                scalar(1, section as u64)
            },
            string(2, &inner),
        ]
        .concat(),
    )
}
const NAMES: [&str; 11] = [
    "Common",
    "Save",
    "UI - Common",
    "UI - Items",
    "Fishing Rods",
    "Boats",
    "Islands",
    "NPCs",
    "Quests",
    "Totems",
    "Shops",
];
fn new_file(path: &Path, b: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(b)?;
    f.sync_all()?;
    Ok(())
}
pub fn make_stage(root: &Path) -> Result<Stage> {
    let stage = root.join("stage/Practice/Beyond_Local_Save_Level/42");
    fs::create_dir_all(stage.join("beyond_level_setting"))?;
    fs::create_dir_all(stage.join("beyond_level_slice"))?;
    new_file(
        &stage.join("42.gil"),
        &frame(
            2,
            [
                scalar(1, 42),
                string(2, b"Fisch - practice"),
                string(43, b"practice-v1"),
            ]
            .concat(),
        ),
    )?;
    let mut rows = vec![];
    for id in 0..64 {
        let name = NAMES
            .get(id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Shared Creation Section {}", id + 1));
        let row = [
            if id == 0 {
                vec![]
            } else {
                scalar(1, id as u64)
            },
            string(2, name.as_bytes()),
        ]
        .concat();
        rows.extend(string(1, &row));
        new_file(
            &stage.join(format!("beyond_level_slice/42_{}.gis", id + 1)),
            &slice(id as u8, &format!("Sample {name} baseline")),
        )?;
    }
    rows.extend(scalar(2, 8));
    new_file(
        &stage.join("beyond_level_setting/local_setting.gils"),
        &frame(8, [scalar(1, 42), string(5, &rows)].concat()),
    )?;
    new_file(
        &stage.join("beyond_level_slice/42.gilh"),
        &frame(
            7,
            [scalar(1, 42), string(90, b"Shared practice baseline")].concat(),
        ),
    )?;
    binary::inspect(&stage)
}
impl Core {
    pub fn is_practice(&self, p: &crate::runtime::LocalProject) -> bool {
        let root = self.data.join("practice/projects");
        p.stage.editor_version == "practice-v1"
            && p.stage.account == "Practice"
            && p.stage
                .path
                .canonicalize()
                .is_ok_and(|path| root.canonicalize().is_ok_and(|r| path.starts_with(r)))
            && self
                .db
                .get::<String>("practice-endpoint")
                .ok()
                .flatten()
                .is_some_and(|ep| ep == p.endpoint)
    }
    /// The test coordinator on this PC, which tests/coordinator.cjs starts.
    pub async fn practice_endpoint(&self) -> Result<String> {
        let endpoint = crate::coordination::TEST_SERVICE_URL.to_string();
        crate::runtime::check_creation_service(&endpoint)
            .await
            .with_context(|| {
                format!(
                    "No test coordinator at {endpoint}. Start one with node tests/coordinator.cjs."
                )
            })?;
        self.db.set("practice-endpoint", &endpoint)?;
        Ok(endpoint)
    }
    /// Opens the practice project, creating it the first time.
    pub async fn begin_practice(&self) -> Result<()> {
        if let Ok(p) = self.active() {
            if self.is_practice(&p) {
                return Ok(());
            }
            self.db.set("practice-return", &p.project.id)?;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.finish()).await;
            let _guard = self.operation.lock().await;
            self.db.remove("active")?;
            *self.active.lock().unwrap() = None;
            *self.watcher.lock().unwrap() = None;
            self.leases.lock().unwrap().clear();
            self.lease_activity.lock().unwrap().clear();
        }
        let endpoint = self.practice_endpoint().await?;
        if let Some(id) = self.db.get::<String>("practice-project")? {
            self.reopen(&id).await?;
            if !self.db.get::<bool>("practice-seeded")?.unwrap_or(false) {
                self.seed_practice().await?;
            }
            return Ok(());
        }
        let root = self
            .data
            .join("practice/projects")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root)?;
        let stage = make_stage(&root)?;
        let exchange = root.join("exchange");
        fs::create_dir(&exchange)?;
        self.create_at(
            "Fisch — practice".into(),
            "You".into(),
            stage.path,
            "folder".into(),
            Some(exchange),
            Some(endpoint),
        )
        .await?;
        self.db
            .set("practice-project", &self.active()?.project.id)?;
        self.seed_practice().await?;
        self.changed();
        Ok(())
    }
    async fn practice_peer(&self) -> Result<String> {
        let p = self.active()?;
        ensure!(self.is_practice(&p), "Open the practice project first");
        let credential = format!("practice-peer:{}", p.project.id);
        if let Ok(token) = crate::oauth::secret_get(&credential) {
            return Ok(token);
        }
        let invitation: Invitation = self
            .api(
                &p,
                reqwest::Method::POST,
                "/invite",
                Some(serde_json::json!({})),
            )
            .await?;
        let peer: Joined = self
            .api(
                &p,
                reqwest::Method::POST,
                "/join",
                Some(
                    serde_json::json!({"invite":invitation.invite,"displayName":"Alex (practice)"}),
                ),
            )
            .await?;
        crate::oauth::secret_set(&credential, &peer.token)?;
        self.db.set(
            &format!("practice-peer-member:{}", p.project.id),
            &peer.member,
        )?;
        for name in ["Mia (practice)", "Sam (practice)"] {
            let _: Joined = self
                .api(
                    &p,
                    reqwest::Method::POST,
                    "/join",
                    Some(serde_json::json!({"invite":invitation.invite,"displayName":name})),
                )
                .await?;
        }
        self.sync(false).await?;
        Ok(peer.token)
    }
    async fn peer_update(
        &self,
        section: &str,
        label: &str,
        summary: &str,
        token: &str,
        heads: &mut BTreeMap<String, Revision>,
    ) -> Result<()> {
        let p = self.active()?;
        ensure!(
            self.is_practice(&p),
            "Practice changes cannot target a game stage"
        );
        let bytes = slice(section.parse()?, label);
        let h = hash(&bytes);
        let update_id = uuid::Uuid::new_v4().to_string();
        let artifact = history_artifact(&update_id, section)?;
        self.storage(&p).put(&artifact, bytes.clone()).await?;
        let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
        let client = reqwest::Client::new();
        let url = format!("{}/projects/{}", p.endpoint, p.project.id);
        let lease: miliastra_core::control::Lease = client
            .post(format!("{url}/locks/acquire"))
            .bearer_auth(token)
            .json(&serde_json::json!({"section":keys.resource(&p.endpoint,&p.project.id,section)?}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let revision = Revision {
            section: section.into(),
            name: crate::names::effective_names(&p.project)
                .get(section)
                .context("Practice section unavailable")?
                .clone(),
            hash: h.clone(),
            content_hash: binary::content_hash(&bytes, section, "42")?,
            parent: heads.get(section).map(|r| r.hash.clone()),
            artifact,
            size: bytes.len() as u64,
            shared_dependency: heads.get("shared").map(|r| r.hash.clone()),
        };
        let req = PublishRequest {
            id: update_id,
            summary: summary.into(),
            details: "Practice update written to the local exchange folder.".into(),
            revisions: vec![revision.clone()],
            fences: BTreeMap::from([(section.into(), lease.fence)]),
            restores: None,
        };
        let mut peer = p.clone();
        peer.member = self
            .db
            .get::<String>(&format!("practice-peer-member:{}", p.project.id))?
            .context("Practice peer unavailable")?;
        let request = self
            .control_command(&peer, "/publish", serde_json::to_value(&req)?)
            .await?;
        let wire: serde_json::Value = client
            .post(format!("{url}/publish"))
            .bearer_auth(token)
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let update: Update =
            serde_json::from_value(self.control_response(&peer, "/publish", wire).await?)?;
        self.storage(&p)
            .put(
                &format!("History/{}/changelog.json", update.id),
                serde_json::to_vec_pretty(&update)?,
            )
            .await?;
        heads.insert(section.into(), revision);
        client
            .post(format!("{url}/locks/release"))
            .bearer_auth(token)
            .json(&serde_json::json!({"section":keys.resource(&p.endpoint,&p.project.id,section)?,"fence":lease.fence}))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    async fn seed_practice(&self) -> Result<()> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let peer = self.practice_peer().await?;
        let mut heads = p.project.heads.clone();
        let ids = ["6", "7", "8", "10", "4", "9", "3", "6"];
        let stories = [
            "Built a sheltered fishing cove",
            "Added harbour dialogue",
            "Connected the lighthouse quest",
            "Clarified shop prices",
            "Balanced starter fishing rods",
            "Connected the northern totems",
            "Polished the item cards",
            "Adjusted the island arrival",
        ];
        for i in 0..79 {
            self.peer_update(
                ids[i % 8],
                &format!("Practice team revision {i}"),
                stories[i % 8],
                &peer,
                &mut heads,
            )
            .await?;
        }
        for id in [5u8, 8] {
            let path = binary::resource_path(&p.stage, &id.to_string())?;
            ensure!(
                path.canonicalize()?
                    .starts_with(self.data.join("practice").canonicalize()?),
                "Practice edit escaped its workspace"
            );
            fs::write(path, slice(id, "Your saved practice edits"))?;
        }
        self.sync(true).await?;
        drop(_guard);
        self.download_pending().await?;
        self.db.set("practice-seeded", &true)?;
        *self.observed.lock().unwrap() = Some(binary::inspect(&p.stage.path)?.settings_version);
        Ok(())
    }
    pub async fn practice_conflict(&self) -> Result<()> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(
            self.is_practice(&p),
            "This action is only for the local practice project"
        );
        ensure!(
            self.installation(&p)?.is_none_or(|r| !r.pending()),
            "Finish applying the pending update first"
        );
        let previous = self.leases.lock().unwrap().get("8").cloned();
        if let Some(lease) = previous {
            let _: serde_json::Value = self
                .api(
                    &p,
                    reqwest::Method::POST,
                    "/locks/release",
                    Some(serde_json::json!({"section":"8","fence":lease.fence})),
                )
                .await?;
            self.leases.lock().unwrap().remove("8");
        }
        let token = self.practice_peer().await?;
        let mut heads = p.project.heads.clone();
        let label = format!("Practice conflict {}", uuid::Uuid::new_v4());
        let mine = binary::resource_path(&p.stage, "8")?;
        ensure!(
            mine.canonicalize()?
                .starts_with(self.data.join("practice").canonicalize()?),
            "Practice edit escaped its workspace"
        );
        fs::write(mine, slice(8, &format!("Your {label}")))?;
        self.peer_update(
            "8",
            &format!("Alex's {label}"),
            "Updated the practice quest",
            &token,
            &mut heads,
        )
        .await?;
        self.sync(false).await?;
        drop(_guard);
        self.download_pending().await?;
        self.changed();
        Ok(())
    }
}
