use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub member: String,
    /// Encrypted on the coordinator; the app decrypts it into an activity object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PresenceStatus>,
}
/// Used both encrypted (coordinator) and decrypted (app).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PresenceStatus {
    Encrypted(String),
    Activity(PresenceActivity),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PresenceActivity {
    pub game: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub version: u32,
    pub endpoint: String,
    pub project: String,
    pub invite: String,
}
