use super::*;

#[test]
fn completion_domain_rejects_zero_and_regression() {
    let domain = HostCompletionDomain::new();
    assert_eq!(
        domain.point(0).unwrap_err().kind(),
        RenderErrorKind::HostContract
    );
    assert_eq!(domain.point(4).unwrap().value(), 4);
    assert_eq!(domain.point(4).unwrap().value(), 4);
    assert_eq!(
        domain.point(3).unwrap_err().kind(),
        RenderErrorKind::HostContract
    );
}

#[test]
fn linux_dma_buf_contract_requires_every_external_extension() {
    let complete = HostedDeviceExtensions {
        external_memory_fd: true,
        external_memory_dma_buf: true,
        image_drm_format_modifier: true,
        external_semaphore_fd: true,
        queue_family_foreign: true,
    };
    assert!(complete.linux_dma_buf_complete());
    assert!(
        !HostedDeviceExtensions {
            external_semaphore_fd: false,
            ..complete
        }
        .linux_dma_buf_complete()
    );
}
