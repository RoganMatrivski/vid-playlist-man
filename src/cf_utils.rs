use worker::KvStore;

pub async fn kv_append(
    kv: &KvStore,
    key: impl AsRef<str>,
    value: impl AsRef<str>,
) -> Result<(), worker::Error> {
    let prev = kv.get(key.as_ref()).text().await?.unwrap_or("".into());
    let newval = prev + value.as_ref();

    kv.put(key.as_ref(), newval)?.execute().await?;

    Ok(())
}
