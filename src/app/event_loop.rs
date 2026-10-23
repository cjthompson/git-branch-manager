use super::*;

impl App {
    // ---- Event Loop ----

    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> std::io::Result<()> {
        // Apply initial sort
        list_state::apply_sort(&mut self.branches, &self.branch_columns);

        // Force a full redraw on the first frame by drawing once, then
        // inserting a resize event so ratatui marks the entire buffer dirty.
        terminal.clear()?;

        // Watchdog: logs to /tmp/gbm-watchdog.log if the main loop stalls >2s
        let tick_ms = Arc::new(AtomicU64::new(now_ms()));
        {
            let watchdog_tick = Arc::clone(&tick_ms);
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let stall = now_ms().saturating_sub(watchdog_tick.load(Ordering::Relaxed));
                if stall > 2000 {
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/gbm-watchdog.log")
                        .and_then(|mut f| {
                            use std::io::Write;
                            writeln!(f, "UI stall: {stall}ms")
                        });
                }
            });
        }

        let mut needs_redraw = true;

        loop {
            tick_ms.store(now_ms(), Ordering::Relaxed);
            if self.drain_channels() {
                needs_redraw = true;
            }

            if needs_redraw {
                terminal.draw(|frame| {
                    self.terminal_rows = frame.area().height;
                    let mut ctx = self.build_render_context();
                    git_branch_manager::ui::render::draw(frame, &mut ctx);
                })?;
                needs_redraw = false;
            }

            if self.should_exit {
                return Ok(());
            }

            if event::poll(Duration::from_millis(50))? {
                let ev = event::read()?;
                needs_redraw = true;
                self.handle_event(ev);
            }
        }
    }

    pub(super) fn build_render_context(&mut self) -> RenderContext<'_> {
        let active_filter_tokens: &[FilterTokenDef] = match self.active_view {
            ViewId::Graph => &[],
            ViewId::Branches => &self.branch_filter_tokens,
            ViewId::Remotes => &self.remote_filter_tokens,
            ViewId::Tags => &self.tag_filter_tokens,
            ViewId::Worktrees => &self.worktree_filter_tokens,
        };

        RenderContext {
            active_view: self.active_view,
            overlay: self.overlay.as_mut(),
            toast: self.toast.as_ref(),
            theme: &self.theme,
            symbols: &self.symbols,
            config: &self.config,
            info_copied_msg: self.info_copied_msg.as_deref(),
            info_hit_regions: &mut self.info_hit_regions,
            info_modal_scroll_offset: &mut self.info_modal_scroll_offset,
            graph: &mut self.graph,
            base_branch: &self.base_branch,
            job_status: self.job_queue.render_data(),
            branches: &mut self.branches,
            remotes: &mut self.remotes,
            tags: &mut self.tags,
            worktrees: &mut self.worktrees,
            branch_columns: &self.branch_columns,
            remote_columns: &self.remote_columns,
            tag_columns: &self.tag_columns,
            worktree_columns: &self.worktree_columns,
            active_filter_tokens,
            render_branch_row,
            render_remote_row,
            render_tag_row,
            render_worktree_row,
        }
    }
}
