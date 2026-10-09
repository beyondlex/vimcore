mod common;
use common::Fixture;

#[test]
#[ignore]
fn debug_at_colon() {
    let mut f = Fixture::new("a\na\n");
    f.feed(["q", "a", "x", "<Esc>", "q"]);
    println!("after rec: {:?}", f.text());
    f.feed([":", "s", "/", "a", "/", "z", "/", "<CR>"]);
    println!("after :s: {:?} statuses {:?}", f.text(), f.host.statuses);
    f.feed(["@", ":"]);
    println!("after @:: {:?} statuses {:?}", f.text(), f.host.statuses);
    f.feed(["@", "@"]);
    println!("after @@: {:?} statuses {:?}", f.text(), f.host.statuses);
}
