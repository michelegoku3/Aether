use crate::local::app_id_from_lua_stem;

#[test]
fn accepts_canonical_and_build_labelled_lua_names() {
    assert_eq!(app_id_from_lua_stem("1158310"), Some(1158310));
    assert_eq!(app_id_from_lua_stem("1158310_12890456"), Some(1158310));
}

#[test]
fn extracts_only_a_leading_app_id() {
    assert_eq!(app_id_from_lua_stem("1158310 (1)"), Some(1158310));
    assert_eq!(app_id_from_lua_stem("CK3_1158310"), None);
    assert_eq!(app_id_from_lua_stem("build_1158310_12890456"), None);
}

#[test]
fn does_not_confuse_build_id_with_selected_app_id() {
    assert_eq!(app_id_from_lua_stem("12890456_1158310"), Some(12890456));
}
