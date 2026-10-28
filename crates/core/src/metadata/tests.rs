use super::*;
#[test]
fn ports_are_scoped_sorted_and_ipv6_is_preserved() {
    let ports = parse_ports(
        "p7\nn127.0.0.1:9000\nn[::1]:8080\np99\nn*:22\np7\nn[::1]:8080\n",
        &HashSet::from([7]),
    );
    assert_eq!(ports.len(), 2);
    assert_eq!(ports[0].url(), "http://[::1]:8080/");
    assert_eq!(ports[1].port, 9000);
}
#[test]
fn external_links_allow_web_urls_only() {
    assert!(http_url("https://github.com/a/b/pull/1").is_ok());
    assert!(http_url("file:///etc/passwd").is_err());
    assert!(http_url("javascript:alert(1)").is_err());
    assert!(http_url("https://user:pass@example.com").is_err());
}
