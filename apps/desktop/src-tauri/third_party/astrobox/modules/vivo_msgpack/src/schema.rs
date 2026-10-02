use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSchema {
    pub class: String,
    pub simple_name: String,
    pub package: Option<String>,
    pub source: Option<String>,
    pub message_kind: Option<String>,
    pub bid: Option<u16>,
    pub cid: Option<u16>,
    pub bid_expr: Option<String>,
    pub cid_expr: Option<String>,
    pub encoding: Option<String>,
    #[serde(default)]
    pub msgpack_fields_effective: Vec<MsgpackFieldSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgpackFieldSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub line: Option<u32>,
    pub msgpack: Option<MsgpackOrder>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MsgpackOrder {
    pub order: i32,
    #[serde(default)]
    pub since: i32,
}

#[derive(Debug, Clone)]
pub struct SchemaCatalog {
    messages: Vec<MessageSchema>,
}

impl SchemaCatalog {
    pub fn from_json_str(json: &str) -> Result<Self> {
        Ok(Self {
            messages: serde_json::from_str(json)?,
        })
    }

    pub fn from_json_slice(json: &[u8]) -> Result<Self> {
        Ok(Self {
            messages: serde_json::from_slice(json)?,
        })
    }

    pub fn messages(&self) -> &[MessageSchema] {
        &self.messages
    }

    pub fn find_by_bid_cid(&self, bid: u16, cid: u16) -> Vec<&MessageSchema> {
        self.messages
            .iter()
            .filter(|msg| msg.bid == Some(bid) && msg.cid == Some(cid))
            .collect()
    }

    pub fn find_by_simple_name(&self, simple_name: &str) -> Option<&MessageSchema> {
        self.messages
            .iter()
            .find(|msg| msg.simple_name == simple_name)
    }
}
