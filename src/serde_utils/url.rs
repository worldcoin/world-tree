use std::borrow::Cow;

use serde::{Deserialize, Serializer};
use url::Url;

pub fn serialize<S>(url: &Url, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&url.to_string())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Cow<'static, str> = Deserialize::deserialize(deserializer)?;

    Url::parse(&s).map_err(serde::de::Error::custom)
}
