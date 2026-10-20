//! The translation table read from a file: `tests/fixtures/zh-hk-sample.tsv`, a small table of made-up keys and
//! texts in the layout of `zh-hk.tsv` (none of it is the game's text).

use ssd_catalogue::translation::{Slot, Translation, page_key, render, string_key};
use std::path::Path;

fn sample() -> Translation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zh-hk-sample.tsv");
    Translation::read(&path).expect("the fixture reads")
}

#[test]
fn the_sample_file_reads_its_rows_and_leaves_out_an_empty_text() {
    let table = sample();
    assert_eq!(table.len(), 6);
    assert_eq!(table.text(&string_key(0, 0)), None, "help:0 has no text: English stays");
    assert_eq!(table.text("nothing:1"), None);
    assert_eq!(table.text(&page_key(&string_key(3, 0), 1)), Some("第二頁。"));
}

#[test]
fn a_same_row_reads_the_row_it_names() {
    let table = sample();
    assert_eq!(table.text("str3:0x01"), table.text("str3:0x00"));
    let text = render(table.text("str3:0x01").unwrap(), |_| String::new());
    assert_eq!(text, "這是一段測試文字，\n用來檢查換行。");
}

#[test]
fn the_slots_are_filled_and_the_engines_codes_left_out() {
    let table = sample();
    let text = render(table.text(&string_key(1, 0)).unwrap(), |slot| match slot {
        Slot::Name(1) => "甲".to_owned(),
        Slot::Name(2) => "乙".to_owned(),
        Slot::Number(1) => "42".to_owned(),
        _ => "?".to_owned(),
    });
    assert_eq!(text, "甲向乙\n發出42點攻擊！");
    assert_eq!(render(table.text("str2:0x001").unwrap(), |_| String::new()), "「你好！」");
}

#[test]
fn a_missing_file_is_an_error_not_a_panic() {
    assert!(Translation::read(Path::new("no/such/zh-hk.tsv")).is_err());
}
