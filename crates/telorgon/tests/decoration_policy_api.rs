use telorgon::app::*;

#[test]
fn explicit_policy_survives_cursor_theme_builder_and_defaults_are_client_preference() {
    let policy = DecorationPolicy {
        negotiation: DecorationNegotiation::PreferServer,
    };
    let compositor = Compositor::new()
        .decoration_policy(policy)
        .cursor_theme(CursorTheme::new());
    assert_eq!(compositor.configured_decoration_policy(), policy);
    assert_eq!(
        Compositor::new().configured_decoration_policy(),
        DecorationPolicy::DEFAULT
    );
}
