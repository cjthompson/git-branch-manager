use super::*;

impl App {
    pub(super) fn primary_code_check_owns_cancel_flag(&self) -> bool {
        self.primary_code_check_request
            .as_ref()
            .zip(self.cancel_flag.as_ref())
            .is_some_and(|(request, current)| Arc::ptr_eq(&request.cancel, current))
    }

    pub(super) fn primary_code_check_is_executing(&self) -> bool {
        self.primary_code_check_owns_cancel_flag()
            && matches!(self.overlay, Some(Overlay::Executing { .. }))
    }

    pub(super) fn clear_cancel_flag_unless_primary_code_check_owns_it(&mut self) {
        if !self.primary_code_check_owns_cancel_flag() {
            self.cancel_flag = None;
        }
    }

    pub(super) fn start_primary_code_check(&mut self, selected_ref: String) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let display_ref = selected_ref
            .strip_prefix("refs/heads/")
            .or_else(|| selected_ref.strip_prefix("refs/remotes/"))
            .or_else(|| selected_ref.strip_prefix("refs/tags/"))
            .unwrap_or(&selected_ref)
            .to_string();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = Arc::clone(&cancel);
        self.primary_code_check_request = Some(PrimaryCodeCheckRequest {
            receiver: rx,
            cancel: Arc::clone(&cancel),
        });
        self.cancel_flag = Some(cancel);
        self.return_view = self.active_view;
        self.info_copied_msg = None;
        self.info_modal_scroll_offset = 0;
        self.toast = None;
        self.overlay = Some(Overlay::Executing {
            label: format!("Checking code from {display_ref} in {base_branch}..."),
            progress: None,
        });
        std::thread::spawn(move || {
            if cancel_for_worker.load(Ordering::Relaxed) {
                return;
            }
            let (primary_reference, primary_oid, outcome) = match git2::Repository::open(&repo_path)
            {
                Ok(repo) => {
                    match git_branch_manager::git::merge_detection::resolve_primary_ref_for_code_check(
                        &repo,
                        &base_branch,
                        Some(&cancel_for_worker),
                    ) {
                        Ok(primary) => {
                            let primary_reference = Some(primary.reference.clone());
                            let primary_oid = Some(primary.oid.to_string());
                            let outcome = git_branch_manager::git::merge_detection::check_ref_code_in_primary(
                                &repo,
                                &primary,
                                &selected_ref,
                                Some(&cancel_for_worker),
                            )
                            .map_err(|error| format!("{error:#}"));
                            (primary_reference, primary_oid, outcome)
                        }
                        Err(error) => (None, None, Err(format!("{error:#}"))),
                    }
                }
                Err(error) => (None, None, Err(format!("{error:#}"))),
            };
            if cancel_for_worker.load(Ordering::Relaxed) {
                return;
            }
            let _ = tx.send(PrimaryCodeCheckResult {
                display_ref,
                base_branch,
                primary_reference,
                primary_oid,
                outcome,
            });
        });
    }
}
