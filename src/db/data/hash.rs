use std::error::Error;

use serde::{Deserialize, Serialize};
use sqlx::encode::IsNull;
use sqlx::postgres::{PgHasArrayType, PgTypeInfo};
use sqlx::Database;

use crate::tree::Hash;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HashWrapper(pub Hash);

impl<'r, DB> sqlx::Decode<'r, DB> for HashWrapper
where
    DB: Database,
    [u8; 32]: sqlx::Decode<'r, DB>,
{
    fn decode(
        value: <DB as Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let bytes = <[u8; 32] as sqlx::Decode<DB>>::decode(value)?;

        let value = Hash::from_be_bytes(bytes);

        Ok(Self(value))
    }
}

impl<'q, DB> sqlx::Encode<'q, DB> for HashWrapper
where
    DB: Database,
    [u8; 32]: sqlx::Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        let bytes = self.0.to_be_bytes();
        <[u8; 32] as sqlx::Encode<DB>>::encode_by_ref(&bytes, buf)
    }
}

impl PgHasArrayType for HashWrapper {
    fn array_type_info() -> PgTypeInfo {
        <[u8; 32] as PgHasArrayType>::array_type_info()
    }
}

impl<DB: Database> sqlx::Type<DB> for HashWrapper
where
    [u8; 32]: sqlx::Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <[u8; 32] as sqlx::Type<DB>>::type_info()
    }

    fn compatible(ty: &DB::TypeInfo) -> bool {
        *ty == Self::type_info()
    }
}
