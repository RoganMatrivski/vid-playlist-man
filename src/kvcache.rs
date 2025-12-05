use anyhow::Result;
use worker::KvStore;

#[derive(Clone)]
pub struct KvCache {
    kv: KvStore,
}

impl KvCache {
    pub fn new(kv: KvStore) -> Self {
        Self { kv }
    }

    pub async fn get_json<T>(&self, key: impl AsRef<str>) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        self.kv
            .get(key.as_ref())
            .json()
            .await
            .map_err(|e: worker::KvError| anyhow::anyhow!("Failed to get kv: {e:?}"))
    }

    pub async fn get_text(&self, key: impl AsRef<str>) -> Result<Option<String>> {
        self.kv
            .get(key.as_ref())
            .text()
            .await
            .map_err(|e: worker::KvError| anyhow::anyhow!("Failed to get kv: {e:?}"))
    }

    pub async fn set<T>(&self, key: impl AsRef<str>, value: T, ttl: u64) -> Result<()>
    where
        T: serde::ser::Serialize,
    {
        self.kv
            .put(key.as_ref(), value)
            .map_err(|e| anyhow::anyhow!("Failed to serialize KV value: {e:?}"))?
            .expiration_ttl(ttl) // 1 week should be fine. No one change stuff that much, right?
            .execute()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to put kv: {e:?}"))
    }

    pub async fn set_text(
        &self,
        key: impl AsRef<str>,
        value: impl ToString,
        ttl: u64,
    ) -> Result<()> {
        self.kv
            .put(key.as_ref(), value.to_string())
            .map_err(|e| anyhow::anyhow!("Failed to serialize KV value: {e:?}"))?
            .expiration_ttl(ttl) // 1 week should be fine. No one change stuff that much, right?
            .execute()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to put kv: {e:?}"))
    }
}
