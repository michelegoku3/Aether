use crate::steam::store::parse_suggest_html;

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
