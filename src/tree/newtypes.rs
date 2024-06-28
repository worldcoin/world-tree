use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

macro_rules! primitive_newtype {
    (pub struct $outer:ident($tname:ty)) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            Serialize,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Deserialize,
        )]
        pub struct $outer(pub $tname);

        impl std::fmt::Display for $outer {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Deref for $outer {
            type Target = $tname;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $outer {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        impl From<$tname> for $outer {
            fn from(value: $tname) -> Self {
                $outer(value)
            }
        }

        impl From<$outer> for $tname {
            fn from(value: $outer) -> Self {
                value.0
            }
        }

        impl From<&$outer> for $tname {
            fn from(value: &$outer) -> Self {
                value.0
            }
        }
    };
}

primitive_newtype!(pub struct ChainId(u64));
primitive_newtype!(pub struct NodeIndex(u32));
primitive_newtype!(pub struct LeafIndex(u32));
