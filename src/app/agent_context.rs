//! The context Herdr gives agents about their space: the other checkouts
//! filed beside theirs, so work that spans repos starts with every path.

use std::collections::HashSet;
use std::path::PathBuf;

use super::state::AppState;
use crate::workspace::Workspace;

/// One checkout in a space, as the agent context names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SpaceCheckout {
    label: String,
    path: PathBuf,
    branch: Option<String>,
    /// Closed members still exist on disk but have no workspace in Herdr.
    open: bool,
}

impl AppState {
    /// The context for an agent starting in workspace `ws_idx`: `None` when
    /// its space is `other`, has agent context turned off, or holds no other
    /// checkout.
    pub(crate) fn agent_context_for_workspace(&self, ws_idx: usize) -> Option<String> {
        let workspace = self.workspaces.get(ws_idx)?;
        let space = self
            .space(&workspace.space_id)
            .filter(|space| !space.is_other() && space.agent_context)?;
        let own = self.live_checkout(workspace);
        let mut seen = HashSet::from([crate::worktree::canonical_or_original(&own.path)]);
        let mut others = Vec::new();
        let live = self
            .workspaces
            .iter()
            .filter(|member| member.space_id == space.id)
            .map(|member| self.live_checkout(member));
        let closed = space.closed.iter().map(|member| SpaceCheckout {
            label: member
                .worktree_space
                .as_ref()
                .map(|membership| membership.label.clone())
                .unwrap_or_else(|| member.label.clone()),
            path: member.cwd.clone(),
            branch: member.branch.clone(),
            open: false,
        });
        for checkout in live.chain(closed) {
            if seen.insert(crate::worktree::canonical_or_original(&checkout.path)) {
                others.push(checkout);
            }
        }
        (!others.is_empty()).then(|| render_agent_context(&space.name, &own, &others))
    }

    fn live_checkout(&self, workspace: &Workspace) -> SpaceCheckout {
        let membership = workspace.worktree_space();
        SpaceCheckout {
            label: membership
                .map(|membership| membership.label.clone())
                .unwrap_or_else(|| workspace.display_name_from_terminals(&self.terminals)),
            path: membership
                .map(|membership| membership.checkout_path.clone())
                .unwrap_or_else(|| workspace.identity_cwd.clone()),
            branch: workspace.branch(),
            open: true,
        }
    }
}

fn describe(checkout: &SpaceCheckout) -> String {
    let mut notes = Vec::new();
    if let Some(branch) = &checkout.branch {
        notes.push(format!("branch {branch}"));
    }
    if !checkout.open {
        notes.push("not open in Herdr".to_owned());
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    };
    format!("{}{notes}: {}", checkout.label, checkout.path.display())
}

fn render_agent_context(space: &str, own: &SpaceCheckout, others: &[SpaceCheckout]) -> String {
    let mut text = format!(
        "Herdr space context: this checkout belongs to the Herdr space \"{space}\", which groups \
         the checkouts for one piece of work across repos.\n\nYou are in {}.\n\nOther checkouts \
         in this space:\n",
        describe(own)
    );
    for checkout in others {
        text.push_str(&format!("- {}\n", describe(checkout)));
    }
    text.push_str(
        "\nWhen the work involves these projects, read and change them at those paths. Each is \
         its own Git checkout, so run Git commands in the matching directory.",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::{ClosedMember, OTHER_SPACE_ID};
    use crate::workspace::WorktreeSpaceMembership;

    fn member(id: &str, space_id: &str, label: &str, checkout: &str) -> Workspace {
        let mut workspace = Workspace::test_new(id);
        workspace.id = id.to_owned();
        workspace.space_id = space_id.to_owned();
        workspace.identity_cwd = checkout.into();
        workspace.worktree_space = Some(WorktreeSpaceMembership {
            key: format!("/code/{label}/.git"),
            label: label.to_owned(),
            repo_root: format!("/code/{label}").into(),
            checkout_path: checkout.into(),
            is_linked_worktree: true,
        });
        workspace
    }

    fn state_with_space() -> (AppState, String) {
        let mut state = AppState::test_new();
        state.normalize_spaces();
        let space = state.create_space("billing").unwrap();
        state.workspaces = vec![
            member("w1", &space, "api", "/wt/billing/api/billing"),
            member("w2", &space, "web", "/wt/billing/web/billing"),
            member("w3", OTHER_SPACE_ID, "docs", "/wt/other/docs/x"),
        ];
        state.normalize_spaces();
        (state, space)
    }

    #[test]
    fn context_names_the_other_checkouts_in_the_space() {
        let (mut state, space) = state_with_space();
        let index = state.space_index(&space).unwrap();
        state.spaces[index].closed.push(ClosedMember::new(
            "infra".into(),
            "/code/infra".into(),
            None,
            Some("main".into()),
            None,
        ));

        let context = state.agent_context_for_workspace(0).expect("context");
        assert!(context.contains("Herdr space \"billing\""), "{context}");
        let line = |prefix: &str| {
            context
                .lines()
                .find(|line| line.starts_with(prefix))
                .unwrap_or_else(|| panic!("no line starting {prefix:?} in {context}"))
                .to_owned()
        };
        assert!(line("You are in api").ends_with(": /wt/billing/api/billing."));
        assert!(line("- web").ends_with(": /wt/billing/web/billing"));
        assert!(
            context.contains("- infra (branch main, not open in Herdr): /code/infra"),
            "{context}"
        );
        assert!(!context.contains("/wt/other"), "{context}");
        assert!(
            !context.contains("- api"),
            "the agent's own checkout is not listed"
        );
    }

    #[test]
    fn no_context_without_other_checkouts_or_when_turned_off() {
        let (mut state, space) = state_with_space();
        assert_eq!(
            state.agent_context_for_workspace(2),
            None,
            "other never groups work"
        );

        // A second workspace on the same checkout is not another checkout.
        state.workspaces[1] = member("w2", &space, "api", "/wt/billing/api/billing");
        assert_eq!(state.agent_context_for_workspace(0), None);

        state.workspaces[1] = member("w2", &space, "web", "/wt/billing/web/billing");
        assert!(state.agent_context_for_workspace(0).is_some());
        assert_eq!(state.set_space_agent_context(&space, false), Ok(true));
        assert_eq!(state.agent_context_for_workspace(0), None);
        assert_eq!(state.set_space_agent_context(&space, false), Ok(false));
        assert_eq!(
            state.set_space_agent_context(OTHER_SPACE_ID, false),
            Err(super::super::spaces::SpaceError::BuiltIn)
        );
    }
}
