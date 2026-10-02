use vivo_msgpack::{
    messages::{
        bind::{BindAuthRequest, BindAuthResponse, DEFAULT_BID_VERSION},
        device_info::{BidVersion, FeatureItem, WatchBidVersionRequest},
        generated::MESSAGE_DESCRIPTORS,
    },
    msgpack::{MsgpackReader, write_bin, write_i32, write_i64, write_str},
};

#[test]
fn bind_auth_payload_uses_documented_order() {
    let sign = vec![1, 2, 3, 4, 5];
    let req = BindAuthRequest {
        open_id: "open-id".to_string(),
        random: 0x1234,
        aes_sign: sign.clone(),
        bid_version: DEFAULT_BID_VERSION,
        magic_phone: String::new(),
    };
    let payload = req.payload().unwrap();
    let mut reader = MsgpackReader::new(&payload);

    assert_eq!(reader.read_str().unwrap(), "open-id");
    assert_eq!(reader.read_i32().unwrap(), 0x1234);
    assert_eq!(reader.read_bin().unwrap(), sign);
    assert_eq!(reader.read_i32().unwrap(), 11);
    assert_eq!(reader.read_str().unwrap(), "***********");
    assert!(!reader.has_next());
}

#[test]
fn codegen_exports_schema_catalog() {
    let bind_auth = MESSAGE_DESCRIPTORS
        .iter()
        .find(|message| message.simple_name == "BindAuthRequest")
        .unwrap();

    assert_eq!(bind_auth.bid, Some(15));
    assert_eq!(bind_auth.cid, Some(1));
    assert_eq!(bind_auth.fields.len(), 0);
}

#[test]
fn bind_auth_response_decodes_fields_in_reflective_order() {
    let sign = vec![9, 8, 7, 6];
    let mut payload = Vec::new();
    write_i32(&mut payload, 0);
    write_str(&mut payload, "SN123").unwrap();
    write_i32(&mut payload, 0x2211);
    write_bin(&mut payload, &sign).unwrap();
    write_i64(&mut payload, 11);
    write_i64(&mut payload, 9988);

    let resp = BindAuthResponse::decode(&payload).unwrap();
    assert_eq!(resp.code, 0);
    assert_eq!(resp.bid_version, 11);
    assert_eq!(resp.seq, 9988);
    assert_eq!(resp.aes_sign, sign);
}

#[test]
fn watch_bid_version_request_omits_absent_feature_list() {
    let req = WatchBidVersionRequest {
        sync_bid_versions: true,
        sync_features: true,
        phone_bid_version: 0,
        bid_versions: vec![BidVersion {
            bid: 25,
            version: 19,
        }],
        features: None,
    };
    let payload = req.payload().unwrap();
    let mut reader = MsgpackReader::new(&payload);

    assert!(reader.read_bool().unwrap());
    assert!(reader.read_bool().unwrap());
    assert_eq!(reader.read_i32().unwrap(), 0);
    assert_eq!(reader.read_array_len().unwrap(), 1);

    let bid_payload = reader.read_bin().unwrap();
    let mut bid_reader = MsgpackReader::new(&bid_payload);
    assert_eq!(bid_reader.read_i32().unwrap(), 25);
    assert_eq!(bid_reader.read_i32().unwrap(), 19);
    assert!(!bid_reader.has_next());
    assert!(!reader.has_next());
}

#[test]
fn watch_bid_version_request_writes_present_feature_list() {
    let req = WatchBidVersionRequest {
        sync_bid_versions: true,
        sync_features: true,
        phone_bid_version: 0,
        bid_versions: Vec::new(),
        features: Some(vec![FeatureItem {
            key: 7,
            value: "on".to_string(),
        }]),
    };
    let payload = req.payload().unwrap();
    let mut reader = MsgpackReader::new(&payload);

    assert!(reader.read_bool().unwrap());
    assert!(reader.read_bool().unwrap());
    assert_eq!(reader.read_i32().unwrap(), 0);
    assert_eq!(reader.read_array_len().unwrap(), 0);
    assert_eq!(reader.read_array_len().unwrap(), 1);

    let feature_payload = reader.read_bin().unwrap();
    let mut feature_reader = MsgpackReader::new(&feature_payload);
    assert_eq!(feature_reader.read_i32().unwrap(), 7);
    assert_eq!(feature_reader.read_str().unwrap(), "on");
    assert!(!feature_reader.has_next());
    assert!(!reader.has_next());
}
