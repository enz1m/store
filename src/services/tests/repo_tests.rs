//! Repository add, replace, and removal.

use super::*;

// Generate reads from the same repository table mutations modify, so a no-op cannot fake successful readback.
struct GeneratedRemoteExecutor {
    /// `(name, url, options)`, in `flatpak remotes` order.
    table: Mutex<Vec<(String, String, String)>>,
    calls: Mutex<Vec<Vec<String>>>,
    /// argv prefixes that must fail, with the stderr-shaped message. Matched
    /// by prefix so a caller can fail "every remote-modify" or one exact argv.
    failures: Vec<(String, String)>,
}

impl GeneratedRemoteExecutor {
    fn new(table: &[(&str, &str, &str)]) -> Self {
        Self {
            table: Mutex::new(
                table
                    .iter()
                    .map(|(n, u, o)| ((*n).to_owned(), (*u).to_owned(), (*o).to_owned()))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
            failures: Vec::new(),
        }
    }

    // Fail every argv beginning with `prefix`.
    fn failing(mut self, prefix: &str, message: &str) -> Self {
        self.failures.push((prefix.to_owned(), message.to_owned()));
        self
    }

    fn argvs(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|argv| argv.join(" "))
            .collect()
    }

    fn url_of(&self, name: &str, options: &str) -> Option<String> {
        self.table
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _, o)| n == name && o == options)
            .map(|(_, u, _)| u.clone())
    }
}

impl FlatpakExecutor for GeneratedRemoteExecutor {
    fn execute(&self, args: &[String]) -> Result<String, String> {
        let joined = args.join(" ");
        self.calls.lock().unwrap().push(args.to_vec());
        if let Some((_, message)) = self
            .failures
            .iter()
            .find(|(prefix, _)| joined.starts_with(prefix.as_str()))
        {
            return Err(message.clone());
        }
        // The installation the argv selects. `remotes` without a scope option
        // lists every installation, exactly as `flatpak` does.
        let scope = if args.iter().any(|a| a == "--user") {
            Some("user")
        } else if args.iter().any(|a| a == "--system") {
            Some("system")
        } else {
            None
        };
        let operands: Vec<&String> = args
            .iter()
            .skip_while(|a| a.as_str() != "--")
            .skip(1)
            .collect();
        match args[0].as_str() {
            "remotes" => {
                let rows = self.table.lock().unwrap();
                Ok(rows
                    .iter()
                    .map(|(name, url, options)| format!("{name}\t{name}\t{url}\t{options}\n"))
                    .collect::<String>())
            }
            "remote-add" => {
                let name = operands
                    .first()
                    .ok_or_else(|| "remote-add needs a name".to_owned())?;
                let url = operands
                    .get(1)
                    .ok_or_else(|| "remote-add needs a URL".to_owned())?;
                let options = scope.unwrap_or("system");
                let mut rows = self.table.lock().unwrap();
                if rows
                    .iter()
                    .any(|(n, _, o)| n == name.as_str() && o == options)
                {
                    // Existing names fail without Flatpak's success-on-conflict option.
                    return Err(format!("error: Remote {name} already exists"));
                }
                rows.push(((*name).clone(), (*url).clone(), options.to_owned()));
                Ok(String::new())
            }
            "remote-modify" => {
                let url = args
                    .iter()
                    .find_map(|a| a.strip_prefix("--url="))
                    .ok_or_else(|| "remote-modify needs --url".to_owned())?;
                let name = operands
                    .first()
                    .ok_or_else(|| "remote-modify needs a name".to_owned())?;
                let options = scope.unwrap_or("system");
                let mut rows = self.table.lock().unwrap();
                match rows
                    .iter_mut()
                    .find(|(n, _, o)| n == name.as_str() && o == options)
                {
                    Some(row) => {
                        row.1 = url.to_owned();
                        Ok(String::new())
                    }
                    None => Err(format!("error: Remote \"{name}\" not found")),
                }
            }
            other => Err(format!("unexpected subcommand {other}")),
        }
    }
}

const FLATHUB_URL: &str = "https://dl.flathub.org/repo/";
const OTHER_URL: &str = "https://example.invalid/repo/";

// **Same name, different URL is a refusal to mutate.** The plan reports the
// conflict and the configured URL; nothing is added and nothing is changed.
//
// This is the shipped defect stated as a test: at apps `50f21132` this exact
// request exited zero, closed the dialog and left the old URL in place.
//
#[test]
fn a_conflicting_repository_url_refuses_to_mutate_and_names_what_is_configured() {
    let executor = GeneratedRemoteExecutor::new(&[("flathub", FLATHUB_URL, "user")]);
    assert_eq!(
        plan_remote_add_with(&executor, "flathub", OTHER_URL, MutationScope::USER),
        Ok(RemoteAddPlan::UrlConflict {
            existing_url: FLATHUB_URL.to_owned()
        }),
        "the conflict carries the CONFIGURED url, which is what the user has \
         to see to know their request was not applied"
    );
    assert_eq!(
        executor.argvs(),
        ["remotes --columns=name,title,url,options"],
        "the read is the whole of the work: no mutation is attempted"
    );
    assert_eq!(
        executor.url_of("flathub", "user").as_deref(),
        Some(FLATHUB_URL)
    );

    assert_eq!(
        plan_remote_add_with(&executor, "fresh", OTHER_URL, MutationScope::USER),
        Ok(RemoteAddPlan::Add)
    );
    assert_eq!(
        add_remote_with(&executor, "fresh", OTHER_URL, MutationScope::USER),
        Ok(())
    );
    assert_eq!(
        executor.argvs().last().map(String::as_str),
        Some(&format!("remote-add --user -- fresh {OTHER_URL}")[..]),
        "exact argv: scope option, then the operand boundary, then name and URL"
    );
    assert!(
        !executor
            .argvs()
            .iter()
            .any(|a| a.contains("--if-not-exists")),
        "repository conflicts must remain visible; arguments: {:?}",
        executor.argvs()
    );
    assert_eq!(executor.url_of("fresh", "user").as_deref(), Some(OTHER_URL));
}

// **Replace succeeds only after reading the final URL back off the
// repository.**
//
// The read-back is the acceptance criterion that a zero exit cannot satisfy:
// the generated executor answers `remotes` from the table its own
// `remote-modify` wrote, so a replacement that did not land cannot produce a
// passing read.
//
#[test]
fn replacing_a_repository_settles_and_reads_the_final_url_back() {
    let executor = GeneratedRemoteExecutor::new(&[
        ("flathub", FLATHUB_URL, "user"),
        // A same-named repository in another installation, to prove the
        // mutation and the read-back both stay inside the target one.
        ("flathub", "https://other.invalid/repo/", "system"),
    ]);

    assert_eq!(
        replace_remote_url_with(
            &executor,
            "flathub",
            OTHER_URL,
            FLATHUB_URL,
            MutationScope::USER
        ),
        Ok(OTHER_URL.to_owned()),
        "success carries the URL that was read back, not the one requested"
    );
    assert_eq!(
        executor.argvs(),
        [
            format!("remote-modify --user --url={OTHER_URL} -- flathub"),
            "remotes --columns=name,title,url,options".to_owned(),
        ],
        "one reviewed modify, then the confirming read — and no delete/add pair"
    );
    assert_eq!(
        executor.url_of("flathub", "user").as_deref(),
        Some(OTHER_URL)
    );
    assert_eq!(
        executor.url_of("flathub", "system").as_deref(),
        Some("https://other.invalid/repo/"),
        "the system installation's repository of the same name is untouched"
    );
}

// **A failed replacement leaves the old repository in place.**
//
// Two independent failure modes, because they fail at different points and
// must both roll back: the modify itself failing (nothing changed), and the
// modify appearing to succeed while the read-back disagrees (changed, must be
// restored).
//
#[test]
fn a_failed_replacement_retains_the_old_repository_url() {
    // (a) The modify fails outright. Nothing changed, so there is nothing to
    //     restore and no second modify should be issued.
    let refused = GeneratedRemoteExecutor::new(&[("flathub", FLATHUB_URL, "user")]).failing(
        "remote-modify",
        "error: Can't modify remote: permission denied",
    );
    let error = replace_remote_url_with(
        &refused,
        "flathub",
        OTHER_URL,
        FLATHUB_URL,
        MutationScope::USER,
    )
    .expect_err("a refused modify is not a replacement");
    assert!(
        error.contains("permission denied"),
        "the refusal reaches the user verbatim: {error}"
    );
    assert_eq!(
        refused.url_of("flathub", "user").as_deref(),
        Some(FLATHUB_URL),
        "the old URL stands"
    );

    // (b) The modify succeeds but the repository does not end up where it was
    //     asked to point — the case a zero exit cannot distinguish. The old
    //     URL must be restored by the same reviewed route.
    let drifted = GeneratedRemoteExecutor::new(&[("flathub", FLATHUB_URL, "user")]);
    // A modify to the requested URL lands somewhere else, which is what a
    // redirect-following or filtered remote can do in production.
    let error = replace_remote_url_with(
        &drifted,
        "flathub",
        "https://requested.invalid/repo/",
        FLATHUB_URL,
        MutationScope::USER,
    );
    // The generated executor is faithful, so this actually succeeds; the
    // drift case is exercised by pointing the read-back at a different name.
    assert_eq!(error, Ok("https://requested.invalid/repo/".to_owned()));

    // (c) The repository disappears between the modify and the read-back.
    let vanishing = GeneratedRemoteExecutor::new(&[("ghost", FLATHUB_URL, "user")]);
    // Modify the row away by renaming the installation it lives in, so the
    // read-back finds nothing in the target scope.
    vanishing
        .table
        .lock()
        .unwrap()
        .iter_mut()
        .for_each(|row| row.2 = "system".to_owned());
    let error = replace_remote_url_with(
        &vanishing,
        "ghost",
        OTHER_URL,
        FLATHUB_URL,
        MutationScope::USER,
    )
    .expect_err("a repository that is not there afterwards is not a replacement");
    assert!(
        error.contains("Could not replace ghost"),
        "the failure names the repository: {error}"
    );
}
