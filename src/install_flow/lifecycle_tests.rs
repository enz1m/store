//! Install and uninstall lifecycle: the action slot, progress, and navigation.

use super::*;

#[test]
fn exact_attempt_cancel_signals_once_and_stays_visible_until_terminal_settlement() {
    let mut store = browse_store();
    drop(store.update(Message::Navigate(Route::Library)));
    let id = OperationId::new("org.test.Hero", 42);
    let cancellation = crate::services::OperationCancellation::default();
    store.operations.push(OperationState::attempt(
        id.app_id.clone(),
        "HeroApp",
        OpKind::Install,
        id.attempt,
    ));
    store
        .operation_cancels
        .insert(id.clone(), cancellation.clone());

    let before = nodes(&store);
    let cancel_before = before
        .iter()
        .find(|node| node.semantics.label.as_deref() == Some("Cancel installing HeroApp"))
        .unwrap_or_else(|| {
            panic!(
                "a live operation exposes its exact Cancel action; labels: {:?}",
                labels(&before)
            )
        });
    assert!(!cancel_before.semantics.disabled);

    drop(
        store.update(Message::CancelOperationRequested(OperationId::new(
            id.app_id.clone(),
            id.attempt - 1,
        ))),
    );
    assert!(!cancellation.is_cancelled_for_bridge());
    assert_eq!(store.operations[0].cancel, CancelState::Idle);

    drop(store.update(Message::CancelOperationRequested(id.clone())));
    assert!(cancellation.is_cancelled_for_bridge());
    assert_eq!(store.operations[0].cancel, CancelState::Cancelling);
    assert_eq!(store.operations[0].message, crate::CANCELLING_MESSAGE);
    let cancelling = nodes(&store);
    let cancel_after = cancelling
        .iter()
        .find(|node| node.semantics.label.as_deref() == Some("Cancel installing HeroApp"))
        .expect("Cancel remains visible while the process group settles");
    assert!(cancel_after.semantics.disabled);

    drop(store.update(Message::CancelOperationRequested(id.clone())));
    assert_eq!(store.operations[0].cancel, CancelState::Cancelling);
    drop(
        store.update(Message::OperationEvent(crate::flatpak::OpEvent::Finished {
            id: id.clone(),
            terminal: crate::flatpak::OperationTerminal::Cancelled,
        })),
    );
    assert!(store.operations[0].finished);
    assert_eq!(store.operations[0].cancel, CancelState::Cancelled);
    assert!(!store.operation_cancels.contains_key(&id));
    assert_eq!(
        nodes(&store)
            .iter()
            .filter(|node| node.semantics.label.as_deref() == Some("Cancel installing HeroApp"))
            .count(),
        0
    );
}

#[test]
fn mutation_lifecycle_worker_panic_becomes_terminal_after_owner_reclaims_slot() {
    let mut store = browse_store();
    let id = OperationId::new("org.test.Panic", 1);
    store.operations.push(OperationState::attempt(
        id.app_id.clone(),
        "Panic",
        OpKind::Install,
        id.attempt,
    ));
    store.operation_cancels.insert(
        id.clone(),
        crate::services::OperationCancellation::default(),
    );
    struct ExitOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for ExitOnDrop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    let worker_exited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let exit = ExitOnDrop(std::sync::Arc::clone(&worker_exited));
    let completion =
        store
            .mutation_owner
            .submit(id.attempt, move |_| -> crate::flatpak::OperationTerminal {
                let _exit = exit;
                panic!("forced mutation worker panic")
            });
    store.operation_completions.insert(
        id.clone(),
        crate::OperationCompletion {
            task: Box::pin(completion),
            progress_drained: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            worker_exited,
        },
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while store.in_flight_operation(&id).is_some() {
        drop(store.update(Message::OperationPoll));
        assert!(
            std::time::Instant::now() < deadline,
            "panicking mutation attempt did not settle within the bounded wait"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let operation = store
        .operations
        .iter()
        .find(|operation| operation.app_id == id.app_id && operation.attempt == id.attempt)
        .expect("panicking attempt remains visible");
    assert!(operation.finished);
    assert_eq!(operation.message, "Failed");
    assert!(
        operation
            .error
            .as_deref()
            .is_some_and(|error| error.contains("ended unexpectedly"))
    );
    assert!(!store.operation_cancels.contains_key(&id));
    assert!(!store.operation_completions.contains_key(&id));
    while store.mutation_owner.dispatcher().snapshot().live() != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "panicking mutation owner did not reclaim the exact worker slot"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn a_completed_uninstall_lets_the_catalogue_offer_install_again() {
    let mut s = browse_store();
    let _ = s.update(Message::InstalledLoaded(Ok(vec![installed_user(
        "org.test.Hero",
        "HeroApp",
    )])));
    s.refresh_projection();
    assert_eq!(
        role_count(&nodes(&s), Role::Button, "Install HeroApp"),
        0,
        "an installed app must not be offered for install"
    );

    let _ = s.update(Message::UninstallConfirmRequested(
        "org.test.Hero".into(),
        "HeroApp".into(),
    ));
    let _ = s.update(Message::UninstallConfirmEvent(
        enzui_widgets::confirm_dialog::ConfirmDialogEvent::Resolved(
            enzui_core::modal::DismissCause::DefaultAction,
        ),
    ));
    let _ = s.update(Message::OperationEvent(crate::flatpak::OpEvent::Finished {
        id: current_operation_id(&s, "org.test.Hero"),
        terminal: crate::flatpak::OperationTerminal::Succeeded,
    }));
    s.refresh_projection();

    assert!(
        !s.is_installed("org.test.Hero"),
        "a completed uninstall must drop the app from the installed list"
    );
    let ns = nodes(&s);
    assert_eq!(
        role_count(&ns, Role::Button, "Install HeroApp"),
        1,
        "the catalogue row must offer install again once the app is gone; \
         labels: {:?}",
        labels(&ns)
    );
}

// Test mutations resolve to NO_FLATPAK_PROGRAM so no test can invoke host Flatpak.
