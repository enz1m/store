//! Installation cells and mutation scope.

use super::*;

#[test]
fn a_fresh_install_resolves_its_repository_and_never_guesses_a_scope() {
    let remote = |name: &str, options: &str| RemoteInfo {
        name: name.into(),
        title: name.into(),
        url: FLATHUB_REPOSITORY.url.into(),
        options: options.into(),
    };
    let user_only = vec![remote("flathub", "user")];
    let system_only = vec![remote("flathub", "system")];
    // The ordinary "added in both scopes" layout, which is exactly why the
    // Repositories page grew a scope caption.
    let both = vec![remote("flathub", "system"), remote("flathub", "user")];
    let loaded = RemoteLoad::Loaded;

    // --- the two positive answers ------------------------------------------
    assert_eq!(
        install_authorization("flathub", &loaded, &user_only),
        Authorization::Allowed(MutationScope::USER),
        "a user repository can supply it"
    );
    assert_eq!(
        install_authorization("flathub", &loaded, &both),
        Authorization::Allowed(MutationScope::USER),
        "with the same repository in both scopes the user one wins — it is the \
         one this build can install from, and the refs are identical"
    );
    assert_eq!(
        install_authorization("flathub", &loaded, &system_only),
        Authorization::Refused(SYSTEM_REMOTE_ONLY),
        "the verified production failure: --user cannot resolve a ref that \
         only a system-wide repository carries"
    );

    // --- every unknown is its own refusal, never an install ----------------
    let no_repositories: Vec<RemoteInfo> = Vec::new();
    let unrelated = vec![RemoteInfo {
        name: "gnome-nightly".into(),
        title: "GNOME Nightly".into(),
        url: "https://nightly.gnome.org/gnome-nightly.flatpakrepo".into(),
        options: "user".into(),
    }];
    for (origin, load, remotes, expected, why) in [
        (
            "flathub",
            RemoteLoad::Loading,
            &user_only,
            REPOSITORIES_LOADING,
            "the read has not answered, so nothing is known yet",
        ),
        (
            "flathub",
            RemoteLoad::Failed("flatpak exited with status 1".into()),
            &user_only,
            REPOSITORIES_UNAVAILABLE,
            "the read failed, so nothing is known at all",
        ),
        (
            "flathub",
            RemoteLoad::Loaded,
            &no_repositories,
            REPOSITORY_UNKNOWN,
            "loaded and empty is an answer: nothing can supply it",
        ),
        (
            "flathub",
            RemoteLoad::Loaded,
            &unrelated,
            REPOSITORY_UNKNOWN,
            "loaded with no matching repository",
        ),
        (
            "",
            RemoteLoad::Loaded,
            &user_only,
            REPOSITORY_UNKNOWN,
            "the row carries no origin, so there is nothing to match on",
        ),
        (
            "   ",
            RemoteLoad::Loaded,
            &user_only,
            REPOSITORY_UNKNOWN,
            "a blank origin is an absent origin",
        ),
    ] {
        assert_eq!(
            install_authorization(origin, &load, remotes),
            Authorization::Refused(expected),
            "{why}: this must be a stated refusal, never a guessed --user"
        );
    }

    // The four unknowns do not share one sentence: a user who is waiting, a
    // user who must retry, and a user who must add a repository need different
    // instructions.
    let distinct = [
        REPOSITORIES_LOADING.label,
        REPOSITORIES_UNAVAILABLE.label,
        REPOSITORY_UNKNOWN.label,
        SYSTEM_REMOTE_ONLY.label,
    ];
    for (i, a) in distinct.iter().enumerate() {
        for b in &distinct[i + 1..] {
            assert_ne!(a, b, "two repository refusals must not share a label");
        }
    }
}
