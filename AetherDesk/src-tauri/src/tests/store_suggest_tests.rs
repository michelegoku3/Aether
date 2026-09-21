use crate::steam::store::{parse_suggest_html, suggest_cache_merge, SteamStoreItem};

#[test]
fn parse_suggest_html_reads_app_name_and_id() {
    let html = r#"
<a class="match" data-ds-appid="292030" href="https://store.steampowered.com/app/292030/">
  <div class="match_name">The Witcher 3: Wild Hunt</div>
  <div class="match_img"><img src="https://example.com/capsule.jpg"></div>
</a>
"#;
    let items = parse_suggest_html(html);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, 292030);
    assert_eq!(items[0].name, "The Witcher 3: Wild Hunt");
    assert!(items[0].image_url.contains("capsule.jpg"));
}

fn item(id: u32, name: &str) -> SteamStoreItem {
    SteamStoreItem {
        id,
        name: name.to_string(),
        image_url: String::new(),
        item_type: Some("app".to_string()),
        price: None,
        metascore: None,
        platforms: None,
        streamingvideo: None,
        controller_support: None,
    }
}

fn ids(items: &[SteamStoreItem]) -> Vec<u32> {
    items.iter().map(|item| item.id).collect()
}

#[test]
fn suggest_merge_keeps_ranked_rows_first_whatever_arrives_first() {
    // Unique key: the cache is process-wide and other tests share it.
    let key = "IT:test merge ordering 4f1c";
    // Tail (SearchApps) lands first — e.g. the typeahead already answered
    // with the ranked rows and the detached task merges late.
    let merged = suggest_cache_merge(key, Vec::new(), vec![item(30, "c"), item(10, "a")]);
    assert_eq!(ids(&merged), vec![30, 10]);
    // Ranked rows must still come first, tail deduplicated by id.
    let merged = suggest_cache_merge(key, vec![item(10, "a"), item(20, "b")], Vec::new());
    assert_eq!(ids(&merged), vec![10, 20, 30]);
    // Same-key merge is idempotent.
    let merged = suggest_cache_merge(key, vec![item(10, "a"), item(20, "b")], vec![item(30, "c")]);
    assert_eq!(ids(&merged), vec![10, 20, 30]);
}

#[test]
fn suggest_merge_with_partial_tail_grows_the_same_entry() {
    let key = "IT:test merge growth 9b2e";
    let first = suggest_cache_merge(key, vec![item(1, "ranked")], Vec::new());
    assert_eq!(ids(&first), vec![1]);
    let grown = suggest_cache_merge(key, Vec::new(), vec![item(1, "ranked"), item(2, "tail")]);
    assert_eq!(ids(&grown), vec![1, 2]);
}
