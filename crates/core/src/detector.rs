use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    LiveUi,
    SavedSettings,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub section: Option<String>,
    pub observed_at: i64,
    pub source: Evidence,
    pub historical: bool,
}
pub fn saved(
    section: Option<String>,
    version: &str,
    previous: Option<&str>,
    at: i64,
) -> Observation {
    Observation {
        source: if section.is_some() {
            Evidence::SavedSettings
        } else {
            Evidence::Unknown
        },
        section,
        observed_at: at,
        historical: previous.is_none_or(|v| v == version),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_is_historical_not_live() {
        let o = saved(Some("5".into()), "hash:100", None, 1);
        assert!(o.historical);
        assert_eq!(o.source, Evidence::SavedSettings);
    }
    #[test]
    fn unchanged_content_saved_again_is_fresh() {
        let o = saved(Some("5".into()), "hash:200", Some("hash:100"), 2);
        assert!(!o.historical);
        assert_eq!(o.section.as_deref(), Some("5"));
    }
    #[test]
    fn repeated_observation_remains_historical() {
        assert!(saved(Some("6".into()), "hash:100", Some("hash:100"), 3).historical);
    }
}
