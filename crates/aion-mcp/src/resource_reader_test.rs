use super::*;

#[test]
fn large_json_is_navigable_without_losing_middle_fields() {
    let text = json!({"cubes": (0..120).map(|i| json!({
        "name": format!("cube_{i}"), "description":"说明".repeat(100),
        "measures":[{"name":format!("cube_{i}.count")}]
    })).collect::<Vec<_>>()})
    .to_string();
    let root: Value = serde_json::from_str(&resource_page(&text, "", 0).unwrap()).unwrap();
    assert_eq!(root["entries"][0]["pointer"], "/cubes");
    let page = resource_page(&text, "/cubes", 40).unwrap();
    assert!(page.len() <= PAGE_BYTES);
    let page: Value = serde_json::from_str(&page).unwrap();
    assert_eq!(page["entries"][0]["name"], "cube_40");
    assert_eq!(page["next_offset"], 60);
    let field: Value = serde_json::from_str(&resource_page(&text, "/cubes/45/measures", 0).unwrap()).unwrap();
    assert_eq!(field["value"][0]["name"], "cube_45.count");
    assert_eq!(field["complete"], true);
}

#[test]
fn escaped_pointers_and_invalid_offsets() {
    let text = json!({"a/b~c": "x".repeat(8000)}).to_string();
    let page: Value = serde_json::from_str(&resource_page(&text, "", 0).unwrap()).unwrap();
    assert_eq!(page["entries"][0]["pointer"], "/a~1b~0c");
    assert!(resource_page(&text, "/a~1b~0c", 0).is_ok());
    assert!(
        resource_page(&text, "/missing", 0)
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
    assert!(
        resource_page(&text, "", 100)
            .unwrap_err()
            .to_string()
            .contains("out of range")
    );
}

#[test]
fn text_pages_reconstruct_unicode_and_remain_bounded() {
    let text = "中文\n\u{0001}\\\"".repeat(1600);
    let mut offset = 0;
    let mut restored = String::new();
    loop {
        let page = resource_page(&text, "", offset).unwrap();
        assert!(page.len() <= PAGE_BYTES);
        let page: Value = serde_json::from_str(&page).unwrap();
        restored.push_str(page["text"].as_str().unwrap());
        if page["next_offset"].is_null() {
            break;
        }
        offset = page["next_offset"].as_u64().unwrap() as usize;
    }
    assert_eq!(restored, text);
}
