use serde::{Deserialize, Serialize, de::DeserializeOwned};
use worker::{
    DurableObject, Env, Error, Method, Request, RequestInit, Response, Result, SqlStorage, State,
    durable_object, wasm_bindgen,
};

#[durable_object]
pub struct StringSet {
    sql: SqlStorage,
}

#[derive(Deserialize, Serialize)]
struct Command {
    operation: String,
    value: Option<String>,
}

#[derive(Deserialize)]
struct Changes {
    changes: i64,
}

#[derive(Deserialize)]
struct Size {
    size: i64,
}

impl StringSet {
    fn did_change(&self) -> Result<bool> {
        let changes: Changes = self.sql.exec("SELECT changes() AS changes", None)?.one()?;
        Ok(changes.changes > 0)
    }

    fn add_value(&self, value: &str) -> Result<bool> {
        self.sql.exec(
            "INSERT OR IGNORE INTO strings (value) VALUES (?)",
            vec![value.into()],
        )?;
        self.did_change()
    }

    fn has_value(&self, value: &str) -> Result<bool> {
        let rows = self.sql.exec(
            "SELECT 1 FROM strings WHERE value = ? LIMIT 1",
            vec![value.into()],
        )?;
        Ok(rows.raw().next().transpose()?.is_some())
    }

    fn delete_value(&self, value: &str) -> Result<bool> {
        self.sql
            .exec("DELETE FROM strings WHERE value = ?", vec![value.into()])?;
        self.did_change()
    }

    fn clear_values(&self) -> Result<()> {
        self.sql.exec("DELETE FROM strings", None)?;
        Ok(())
    }

    fn size_value(&self) -> Result<f64> {
        let size: Size = self
            .sql
            .exec("SELECT COUNT(*) AS size FROM strings", None)?
            .one()?;
        Ok(size.size as f64)
    }
}

impl DurableObject for StringSet {
    fn new(state: State, _env: Env) -> Self {
        let sql = state.storage().sql();
        sql.exec(
            "CREATE TABLE IF NOT EXISTS strings (
              value TEXT PRIMARY KEY
            );",
            None,
        )
        .expect("initialize StringSet schema");
        Self { sql }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let command: Command = req.json().await?;
        match command.operation.as_str() {
            "add" => {
                let value = command
                    .value
                    .ok_or_else(|| Error::from("StringSet.add requires a value"))?;
                Response::from_json(&self.add_value(&value)?)
            }
            "has" => {
                let value = command
                    .value
                    .ok_or_else(|| Error::from("StringSet.has requires a value"))?;
                Response::from_json(&self.has_value(&value)?)
            }
            "delete" => {
                let value = command
                    .value
                    .ok_or_else(|| Error::from("StringSet.delete requires a value"))?;
                Response::from_json(&self.delete_value(&value)?)
            }
            "clear" => {
                self.clear_values()?;
                Response::from_json(&true)
            }
            "size" => Response::from_json(&self.size_value()?),
            _ => Response::error("Unknown StringSet operation", 404),
        }
    }
}

async fn invoke<T: DeserializeOwned>(env: &Env, operation: &str, value: Option<&str>) -> Result<T> {
    let command = Command {
        operation: operation.to_owned(),
        value: value.map(str::to_owned),
    };
    let body = serde_json::to_string(&command).map_err(Error::from)?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_body(Some(worker::wasm_bindgen::JsValue::from_str(&body)));
    let request = Request::new_with_init("https://string-set.internal/", &init)?;
    let mut response = env
        .durable_object("STRING_SET")?
        .get_by_name("global")?
        .fetch_with_request(request)
        .await?;

    if response.status_code() != 200 {
        return Err(Error::from(format!(
            "StringSet.{operation} returned HTTP {}",
            response.status_code()
        )));
    }

    response.json().await
}

pub async fn add(env: &Env, value: &str) -> Result<bool> {
    invoke(env, "add", Some(value)).await
}

pub async fn has(env: &Env, value: &str) -> Result<bool> {
    invoke(env, "has", Some(value)).await
}

pub async fn delete(env: &Env, value: &str) -> Result<bool> {
    invoke(env, "delete", Some(value)).await
}

pub async fn clear(env: &Env) -> Result<()> {
    let _: bool = invoke(env, "clear", None).await?;
    Ok(())
}

pub async fn size(env: &Env) -> Result<f64> {
    invoke(env, "size", None).await
}
