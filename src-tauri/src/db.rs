use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};
pub struct Db(Mutex<Connection>);
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryFilter {
    pub section: Option<String>,
    pub before: Option<u64>,
}
impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS history(project TEXT NOT NULL,id TEXT NOT NULL,sequence INTEGER NOT NULL,document TEXT NOT NULL,PRIMARY KEY(project,id)); CREATE INDEX IF NOT EXISTS history_order ON history(project,sequence DESC);")?;
        c.execute_batch("DROP TABLE IF EXISTS history_search;")?;
        Ok(Self(Mutex::new(c)))
    }
    pub fn get<T: DeserializeOwned>(&self, k: &str) -> Result<Option<T>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT value FROM kv WHERE key=?1")?;
        let mut r = s.query([k])?;
        Ok(if let Some(row) = r.next()? {
            Some(serde_json::from_str(&row.get::<_, String>(0)?)?)
        } else {
            None
        })
    }
    pub fn set<T: Serialize>(&self, k: &str, v: &T) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![k,serde_json::to_string(v)?])?;
        Ok(())
    }
    pub fn update<T: DeserializeOwned + Serialize + Default>(
        &self,
        key: &str,
        change: impl FnOnce(&mut T) -> Result<()>,
    ) -> Result<()> {
        use rusqlite::OptionalExtension;
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        let value: Option<String> = tx
            .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
            .optional()?;
        let mut value: T = value
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default();
        change(&mut value)?;
        tx.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(&value)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn remove(&self, k: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM kv WHERE key=?1", [k])?;
        Ok(())
    }
    pub fn forget_connection(&self, project: &str, endpoint: &str) -> Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        let mut connections: Vec<serde_json::Value> = tx
            .query_row("SELECT value FROM kv WHERE key='connections'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default();
        connections.retain(|v| v["project"]["id"] != project || v["endpoint"] != endpoint);
        tx.execute("INSERT INTO kv(key,value) VALUES('connections',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&connections)?])?;
        let removed = tx.execute(
            "DELETE FROM kv WHERE key='practice-project' AND value=?1",
            [serde_json::to_string(project)?],
        )?;
        if removed > 0 {
            tx.execute("DELETE FROM kv WHERE key='practice-seeded'", [])?;
        }
        tx.execute(
            "DELETE FROM kv WHERE key IN ('active','practice-return')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn raw_values_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT value FROM kv WHERE substr(key,1,length(?1))=?1")?;
        let rows = s.query_map([prefix], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    pub fn cache(&self, p: &str, u: &miliastra_core::Update) -> Result<()> {
        use rusqlite::OptionalExtension;
        let document = serde_json::to_string(u)?;
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT document FROM history WHERE project=? AND id=?",
                params![p, u.id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.as_deref() == Some(document.as_str()) {
            return Ok(());
        }
        tx.execute("INSERT INTO history(project,id,sequence,document) VALUES(?1,?2,?3,?4) ON CONFLICT(project,id) DO UPDATE SET document=excluded.document",params![p,u.id,u.sequence,document])?;
        tx.commit()?;
        Ok(())
    }
    pub fn history_page(
        &self,
        p: &str,
        filter: &HistoryFilter,
    ) -> Result<miliastra_core::HistoryPage> {
        let c = self.0.lock().unwrap();
        let mut sql = c.prepare("SELECT h.document FROM history h WHERE h.project=?1
            AND (?2 IS NULL OR h.sequence<?2)
            AND (?3 IS NULL OR ?3='' OR EXISTS(SELECT 1 FROM json_each(h.document,'$.revisions') WHERE json_extract(value,'$.section')=?3)
                OR EXISTS(SELECT 1 FROM json_each(h.document,'$.nameChanges') WHERE json_extract(value,'$.section')=?3))
            ORDER BY h.sequence DESC LIMIT 51")?;
        let mut entries: Vec<miliastra_core::Update> = sql
            .query_map(params![p, filter.before, filter.section], |r| {
                r.get::<_, String>(0)
            })?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect::<Result<_>>()?;
        let more = entries.len() > 50;
        entries.truncate(50);
        let next = if more {
            entries.last().map(|u| u.sequence)
        } else {
            None
        };
        Ok(miliastra_core::HistoryPage { entries, next })
    }
    pub fn history(&self, p: &str) -> Result<Vec<miliastra_core::Update>> {
        let c = self.0.lock().unwrap();
        let mut s =
            c.prepare("SELECT document FROM history WHERE project=?1 ORDER BY sequence DESC")?;
        let rows = s.query_map([p], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_history_keeps_slice_filtering_cursor_pagination_and_project_isolation() {
        let db = Db(Mutex::new(Connection::open_in_memory().unwrap()));
        db.0.lock().unwrap().execute_batch("CREATE TABLE history(project TEXT,id TEXT,sequence INTEGER,document TEXT,PRIMARY KEY(project,id));
            CREATE INDEX history_order ON history(project,sequence DESC);
            ").unwrap();
        for n in 1..=110 {
            let update = miliastra_core::Update {
                id: uuid::Uuid::new_v4().to_string(),
                sequence: n,
                at: n as i64,
                author: if n % 2 == 0 { "Alex" } else { "Sam" }.into(),
                author_id: if n % 2 == 0 { "a" } else { "b" }.into(),
                summary: format!("Private boats update {n}"),
                details: "Private changelog".into(),
                revisions: vec![],
                restores: None,
                name_changes: vec![miliastra_core::NameChange {
                    section: if n % 2 == 0 { "5" } else { "6" }.into(),
                    before: "Old".into(),
                    after: "New".into(),
                }],
                names_parent: Some(n - 1),
            };
            db.cache("project", &update).unwrap();
            db.cache("project", &update).unwrap();
        }
        let first = db
            .history_page("project", &HistoryFilter::default())
            .unwrap();
        assert_eq!(first.entries.len(), 50);
        assert_eq!(first.entries[0].sequence, 110);
        assert_eq!(first.next, Some(61));
        let second = db
            .history_page(
                "project",
                &HistoryFilter {
                    before: first.next,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(second.entries.len(), 50);
        assert_eq!(second.entries[0].sequence, 60);
        assert_eq!(second.next, Some(11));
        let third = db
            .history_page(
                "project",
                &HistoryFilter {
                    before: second.next,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(third.entries.len(), 10);
        assert!(third.next.is_none());
        let filtered = db
            .history_page(
                "project",
                &HistoryFilter {
                    section: Some("5".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(filtered.entries.len(), 50);
        assert!(filtered
            .entries
            .iter()
            .all(|u| u.name_changes[0].section == "5"));
        let next = db
            .history_page(
                "project",
                &HistoryFilter {
                    section: Some("5".into()),
                    before: filtered.next,
                },
            )
            .unwrap();
        assert_eq!(next.entries.len(), 5);
        assert!(next
            .entries
            .iter()
            .all(|u| u.name_changes[0].section == "5"));
        assert!(db
            .history_page("other-project", &HistoryFilter::default())
            .unwrap()
            .entries
            .is_empty());
        for field in ["q", "author", "from", "to"] {
            assert!(
                serde_json::from_value::<HistoryFilter>(serde_json::json!({field:"removed"}))
                    .is_err()
            );
        }
        assert_eq!(db.history("project").unwrap().len(), 110);
        assert_eq!(db.0.lock().unwrap().query_row::<i64,_,_>(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name LIKE '%search%'", [], |r|r.get(0)
        ).unwrap(), 0);
    }
    #[test]
    fn leaving_removes_only_the_connection_and_keeps_files_drafts_and_tasks_after_restart() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("collab.db");
        std::fs::write(root.join("slice.gis"), b"creator work").unwrap();
        {
            let d = Db::open(&path).unwrap();
            let connections = serde_json::json!([{"project":{"id":"a"},"endpoint":"local"},{"project":{"id":"b"},"endpoint":"local"}]);
            d.set("connections", &connections).unwrap();
            d.set("active", &"a").unwrap();
            d.set("practice-project", &"a").unwrap();
            d.set("practice-seeded", &true).unwrap();
            d.set("draft:a", &"private changelog").unwrap();
            d.set(
                "task:a",
                &serde_json::json!({"kind":"apply","selected":["5"],"manualSelection":true}),
            )
            .unwrap();
            d.forget_connection("a", "local").unwrap();
        }
        {
            let d = Db::open(&path).unwrap();
            let remaining = d
                .get::<Vec<serde_json::Value>>("connections")
                .unwrap()
                .unwrap();
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0]["project"]["id"], "b");
            assert!(d.get::<String>("active").unwrap().is_none());
            assert!(d.get::<String>("practice-project").unwrap().is_none());
            assert!(d.get::<bool>("practice-seeded").unwrap().is_none());
            assert_eq!(
                d.get::<String>("draft:a").unwrap().unwrap(),
                "private changelog"
            );
            assert_eq!(
                d.get::<serde_json::Value>("task:a").unwrap().unwrap()["kind"],
                "apply"
            );
        }
        assert_eq!(
            std::fs::read(root.join("slice.gis")).unwrap(),
            b"creator work"
        );
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(uuid::Uuid::parse_str(&resolved.file_name().unwrap().to_string_lossy()).is_ok());
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn removing_search_index_preserves_history_and_private_drafts() {
        let path = std::env::temp_dir().join(format!("{}.db", uuid::Uuid::new_v4()));
        {
            let db = Db::open(&path).unwrap();
            db.set("draft", &"Unshared work").unwrap();
            db.0.lock()
                .unwrap()
                .execute_batch(
                    "CREATE VIRTUAL TABLE history_search USING fts5(project,id,text);
                 INSERT INTO history_search VALUES('p','u','Old indexed text');
                 INSERT INTO history VALUES('p','u',1,'{\"id\":\"preserved\"}');",
                )
                .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let c = db.0.lock().unwrap();
        assert_eq!(
            c.query_row::<String, _, _>(
                "SELECT document FROM history WHERE project='p'",
                [],
                |r| r.get(0)
            )
            .unwrap(),
            r#"{"id":"preserved"}"#
        );
        assert_eq!(
            c.query_row::<i64, _, _>(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'history_search%'",
                [],
                |r| r.get(0)
            )
            .unwrap(),
            0
        );
        drop(c);
        assert_eq!(db.get::<String>("draft").unwrap().unwrap(), "Unshared work");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn drafts_survive_reopen() {
        let p = std::env::temp_dir().join(format!("{}.db", uuid::Uuid::new_v4()));
        {
            let d = Db::open(&p).unwrap();
            d.set("draft", &"Keep this changelog").unwrap();
        }
        let d = Db::open(&p).unwrap();
        assert_eq!(
            d.get::<String>("draft").unwrap().unwrap(),
            "Keep this changelog"
        );
        drop(d);
        let _ = std::fs::remove_file(p);
    }
}
