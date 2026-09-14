//! CI drift check: shelf IDs must match across catalog MD, v0 pack, and scanner.

use spindle_core::style::antislop::{
    DEFAULT_CATALOG_MARKDOWN, DEFAULT_PACK_TOML, NON_PORTS, SCANNER_SHELF_IDS, ShelfPack,
    catalog_shelf_ids,
};

#[test]
fn shelf_ids_match_across_catalog_pack_and_scanner() {
    let pack = ShelfPack::from_toml(DEFAULT_PACK_TOML).expect("embedded pack");
    let catalog = catalog_shelf_ids(DEFAULT_CATALOG_MARKDOWN);
    let pack_ids = pack.shelf_ids();
    let scanner: Vec<&str> = SCANNER_SHELF_IDS.to_vec();
    assert_eq!(
        catalog, pack_ids,
        "catalog markdown headings drifted from the v0 pack"
    );
    assert_eq!(
        catalog,
        scanner
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>(),
        "scanner SCANNER_SHELF_IDS drifted from the catalog"
    );
    assert_eq!(catalog.len(), 12, "twelve fiction shelves");
    assert!(
        catalog.iter().any(|id| id == "solitary_fade"),
        "solitary_fade ID must stay stable"
    );
    assert_eq!(
        pack.shelf("said_bookism").map(|s| s.severity),
        Some(spindle_core::style::antislop::Severity::Soft)
    );
    for id in NON_PORTS {
        assert!(
            pack.shelf(id).is_none(),
            "{id} is a non-port and must not appear in the pack"
        );
        assert!(!catalog.iter().any(|shelf| shelf == id));
        assert!(!SCANNER_SHELF_IDS.contains(id));
    }
}
