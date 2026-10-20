//! The coordinator protocol. Names, changelogs and file details live in team storage.
use crate::{Invitation, Presence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
macro_rules! record {
 ($name:ident {$($field:ident:$ty:ty),* $(,)?}) => {
 #[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
 #[serde(rename_all="camelCase",deny_unknown_fields)]
 pub struct $name {$(pub $field:$ty),*}
 };
}
record!(Revision {section:String,revision:String,parent:Option<String>,shared_dependency:Option<String>});
record!(PublishRequest {id:String,manifest_hash:String,revisions:Vec<Revision>,fences:BTreeMap<String,u64>,restores:Option<String>});
record!(PublishNames {id:String,manifest_hash:String,parent_version:u64,sections:Vec<String>});
record!(Update {id:String,author_id:String,manifest_hash:String,at:i64,sequence:u64,revisions:Vec<Revision>,restores:Option<String>,name_sections:Vec<String>,names_parent:Option<u64>});
record!(Member {
    id: String,
    profile_hash: String,
    owner: bool,
    joined: i64
});
record!(Lease {
    section: String,
    member: String,
    fence: u64,
    expires: i64
});
record!(CreateProject {id:String,member_id:String,profile_hash:String,project_hash:String,bootstrap:String,baseline:Vec<Revision>,baseline_id:String,manifest_hash:String});
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectView {
    pub id: String,
    pub project_hash: String,
    pub members: Vec<Member>,
    pub locks: Vec<Lease>,
    pub heads: BTreeMap<String, Revision>,
    pub sequence: u64,
    pub names_version: u64,
    pub presence: Vec<Presence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Joined {
    pub project: ProjectView,
    pub member: String,
    pub token: String,
    pub invitation: Option<Invitation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryPage {
    pub entries: Vec<Update>,
    pub next: Option<u64>,
}
pub fn fingerprint(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid manifest fingerprint"
    );
    Ok(())
}
pub fn revision(value: &Revision) -> anyhow::Result<()> {
    anyhow::ensure!(crate::opaque::resource(&value.section), "Unknown section");
    for id in std::iter::once(&value.revision)
        .chain(value.parent.iter())
        .chain(value.shared_dependency.iter())
    {
        anyhow::ensure!(
            uuid::Uuid::parse_str(id)?.to_string() == *id,
            "Invalid revision identifier"
        );
    }
    anyhow::ensure!(
        value.parent.as_ref() != Some(&value.revision),
        "A revision cannot be its own parent"
    );
    anyhow::ensure!(
        value.section != "shared" || value.shared_dependency.is_none(),
        "Shared settings cannot depend on themselves"
    );
    Ok(())
}
pub fn bootstrap(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() <= 4096,
        "Invitation storage locator is too large"
    );
    crate::opaque::field(value, "storage-bootstrap")
}
