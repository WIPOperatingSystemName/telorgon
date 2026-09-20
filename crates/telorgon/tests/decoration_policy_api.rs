use telorgon::app::*;

#[test]
fn explicit_policy_survives_cursor_theme_builder_and_defaults_are_client_preference() {
    let policy = DecorationPolicy {
        negotiation: DecorationNegotiation::ClientPreference,
        title_bar: TitleBarPolicy::Automatic,
        outer_frame: OuterFramePolicy {
            border: FramePartPolicy::Always,
            rounded_clip: FramePartPolicy::Never,
            shadow: FramePartPolicy::Automatic,
        },
        interaction: FrameInteractionPolicy {
            resize_regions: ResizeRegionPolicy::Disabled,
        },
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
