mod common;
use common::Fixture;

#[test]
fn debug_mapping_jG() {
    let mut f = Fixture::new("one\ntwo\nthree\nfour\nfive\n");
    f.vim
        .keymaps
        .map_str_noremap(vimcore::keymap::ModeClass::Normal, "j", "G", true);
    f.feed(["g", "g"]);
    f.feed(["g", "j"]);
    println!("engine gj with j->G: offset={}", f.vim.cursor_offset());
    println!("expect vim: G fires on last line (offset of 'five')");
}
