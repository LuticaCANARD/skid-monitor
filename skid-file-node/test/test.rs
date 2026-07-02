use super::*;

#[test]
fn parses_labeled_root() {
    let root = parse_root("logs=/var/log").unwrap();

    assert_eq!(root.label, "logs");
    assert_eq!(root.path, PathBuf::from("/var/log"));
}

#[test]
fn rejects_unlabeled_root() {
    assert!(parse_root("/var/log").is_none());
    assert!(parse_root("logs=").is_none());
}
