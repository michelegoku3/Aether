use crate::providers::manifest_source::ManifestSource;
use crate::providers::hubcap::HubcapClient;
use crate::providers::ryuu::RyuuClient;
use crate::providers::luatools::LuaToolsClient;
use crate::providers::oureveryday::OureverydayClient;

#[test]
fn test_manifest_source_metadata() {
    let hubcap = HubcapClient::new("test-key".to_string());
    assert_eq!(hubcap.source_id(), "hubcap");
    assert_eq!(hubcap.display_name(), "Hubcap");

    let ryuu = RyuuClient::new("test-key".to_string());
    assert_eq!(ryuu.source_id(), "ryuu");
    assert_eq!(ryuu.display_name(), "Ryuu");

    let luatools = LuaToolsClient::new();
    assert_eq!(luatools.source_id(), "luatools");
    assert_eq!(luatools.display_name(), "LuaTools");

    let moed = OureverydayClient::new();
    assert_eq!(moed.source_id(), "oureveryday");
    assert_eq!(moed.display_name(), "MOED");
}

#[test]
fn test_manifest_source_trait_object() {
    let sources: Vec<Box<dyn ManifestSource>> = vec![
        Box::new(HubcapClient::new("key".to_string())),
        Box::new(RyuuClient::new("key".to_string())),
        Box::new(LuaToolsClient::new()),
        Box::new(OureverydayClient::new()),
    ];

    let ids: Vec<&'static str> = sources.iter().map(|s| s.source_id()).collect();
    assert_eq!(ids, vec!["hubcap", "ryuu", "luatools", "oureveryday"]);
}
