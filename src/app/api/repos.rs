use crate::api::schema::{
    EmptyParams, RepoAddParams, RepoInfo, RepoSettingsSetParams, RepoTarget, RepoUpdateParams,
    ResponseResult,
};
use crate::app::App;
use crate::repos::{Repo, RepoError};

use super::responses::{encode_error, encode_success};

fn repo_error(id: String, error: RepoError) -> String {
    encode_error(id, error.code, error.message)
}

impl App {
    fn repo_infos(&self) -> Vec<RepoInfo> {
        self.state.repos.iter().map(Repo::info).collect()
    }

    /// Applies `repos` and persists them; the in-memory list only changes
    /// when the file write succeeds so the two never disagree.
    fn commit_repos(&mut self, repos: Vec<Repo>) -> Result<(), RepoError> {
        if let Some(path) = self.repos_path.as_deref() {
            crate::repos::save(path, &repos).map_err(|err| RepoError {
                code: "repo_save_failed",
                message: format!("couldn't save {}: {err}", path.display()),
            })?;
        }
        self.state.repos = repos;
        Ok(())
    }

    pub(super) fn handle_repo_list(&self, id: String, _params: EmptyParams) -> String {
        encode_success(
            id,
            ResponseResult::RepoList {
                repos: self.repo_infos(),
            },
        )
    }

    pub(super) fn handle_repo_add(&mut self, id: String, params: RepoAddParams) -> String {
        match self.add_repo(params) {
            Ok(repo) => encode_success(id, ResponseResult::RepoInfo { repo }),
            Err(error) => repo_error(id, error),
        }
    }

    fn add_repo(&mut self, params: RepoAddParams) -> Result<RepoInfo, RepoError> {
        let path = crate::repos::typed_root(&params.root);
        let inspected = crate::repos::inspect_root(&path, params.remote.as_deref())?;
        let root_key = crate::worktree::canonical_or_original(&inspected.root);
        if let Some(existing) = self
            .state
            .repos
            .iter()
            .find(|repo| crate::worktree::canonical_or_original(&repo.root_path()) == root_key)
        {
            return Err(RepoError {
                code: "duplicate_repo_root",
                message: format!("this checkout is already added as {}", existing.name),
            });
        }
        let name = crate::repos::validated_name(
            &self.state.repos,
            params.name.as_deref().unwrap_or(&inspected.name),
            None,
        )?;
        let base_branch = match params.base_branch.as_deref().map(str::trim) {
            Some(base) if !base.is_empty() => crate::repos::validated_branch(base, "base branch")?,
            _ => inspected.base_branch,
        };
        let repo = Repo {
            name,
            root: crate::repos::display_root(&inspected.root),
            base_branch,
            remote: inspected.remote,
            settings: Default::default(),
        };
        let info = repo.info();
        let mut repos = self.state.repos.clone();
        repos.push(repo);
        self.commit_repos(repos)?;
        Ok(info)
    }

    pub(super) fn handle_repo_update(&mut self, id: String, params: RepoUpdateParams) -> String {
        match self.update_repo(params) {
            Ok(repo) => encode_success(id, ResponseResult::RepoInfo { repo }),
            Err(error) => repo_error(id, error),
        }
    }

    fn update_repo(&mut self, params: RepoUpdateParams) -> Result<RepoInfo, RepoError> {
        let index = crate::repos::position(&self.state.repos, &params.repo)
            .ok_or_else(|| RepoError::not_found(&params.repo))?;
        let mut repo = self.state.repos[index].clone();
        if let Some(name) = params.name.as_deref() {
            repo.name = crate::repos::validated_name(&self.state.repos, name, Some(index))?;
        }
        if let Some(root) = params.root.as_deref().map(str::trim) {
            let path = crate::repos::typed_root(root);
            let inspected = crate::repos::inspect_root(&path, repo.remote.as_deref())?;
            repo.root = crate::repos::display_root(&inspected.root);
        }
        if let Some(remote) = params.remote.as_deref().map(str::trim) {
            repo.remote = (!remote.is_empty()).then(|| remote.to_owned());
        }
        if let Some(base) = params.base_branch.as_deref() {
            repo.base_branch = crate::repos::validated_branch(base, "base branch")?;
        }
        let info = repo.info();
        let mut repos = self.state.repos.clone();
        repos[index] = repo;
        self.commit_repos(repos)?;
        Ok(info)
    }

    pub(super) fn handle_repo_settings_set(
        &mut self,
        id: String,
        params: RepoSettingsSetParams,
    ) -> String {
        let result = crate::repos::position(&self.state.repos, &params.repo)
            .ok_or_else(|| RepoError::not_found(&params.repo))
            .and_then(|index| {
                let mut repos = self.state.repos.clone();
                repos[index].settings = crate::repos::validated_settings(params.settings)?;
                let info = repos[index].info();
                self.commit_repos(repos)?;
                Ok(info)
            });
        match result {
            Ok(repo) => encode_success(id, ResponseResult::RepoInfo { repo }),
            Err(error) => repo_error(id, error),
        }
    }

    pub(super) fn handle_repo_remove(&mut self, id: String, target: RepoTarget) -> String {
        let Some(index) = crate::repos::position(&self.state.repos, &target.repo) else {
            return repo_error(id, RepoError::not_found(&target.repo));
        };
        let mut repos = self.state.repos.clone();
        repos.remove(index);
        match self.commit_repos(repos) {
            Ok(()) => self.handle_repo_list(id, EmptyParams::default()),
            Err(error) => repo_error(id, error),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{
        EmptyParams, Method, RepoAddParams, RepoTarget, RepoUpdateParams, Request,
    };
    use crate::app::App;
    use crate::config::Config;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn call(app: &mut App, method: Method) -> serde_json::Value {
        let response = app.handle_api_request(Request {
            id: "req".into(),
            method,
        });
        serde_json::from_str(&response).unwrap()
    }

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "herdr-api-repo-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["init", "--quiet", "--initial-branch=main"])
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    #[test]
    fn repos_are_added_updated_listed_and_removed() {
        let mut app = test_app();
        let root = temp_repo("crud");
        let file = root.join("repos.toml");
        app.repos_path = Some(file.clone());

        let added = call(
            &mut app,
            Method::RepoAdd(RepoAddParams {
                root: root.display().to_string(),
                name: Some("pyshiftup".into()),
                ..RepoAddParams::default()
            }),
        );
        assert_eq!(added["result"]["repo"]["name"], "pyshiftup", "{added}");
        assert_eq!(added["result"]["repo"]["base_branch"], "main");
        assert!(added["result"]["repo"].get("remote").is_none());
        assert!(file.exists(), "repos persist on change");

        let duplicate = call(
            &mut app,
            Method::RepoAdd(RepoAddParams {
                root: root.display().to_string(),
                ..RepoAddParams::default()
            }),
        );
        assert_eq!(duplicate["error"]["code"], "duplicate_repo_root");

        let updated = call(
            &mut app,
            Method::RepoUpdate(RepoUpdateParams {
                repo: "PYSHIFTUP".into(),
                name: Some("shiftup".into()),
                base_branch: Some("develop".into()),
                remote: Some("upstream".into()),
                ..RepoUpdateParams::default()
            }),
        );
        assert_eq!(updated["result"]["repo"]["name"], "shiftup", "{updated}");
        assert_eq!(updated["result"]["repo"]["base_branch"], "develop");
        assert_eq!(updated["result"]["repo"]["remote"], "upstream");
        assert_eq!(crate::repos::load(&file), app.state.repos);

        let listed = call(&mut app, Method::RepoList(EmptyParams::default()));
        assert_eq!(listed["result"]["repos"].as_array().unwrap().len(), 1);

        let removed = call(
            &mut app,
            Method::RepoRemove(RepoTarget {
                repo: "shiftup".into(),
            }),
        );
        assert_eq!(removed["result"]["repos"], serde_json::json!([]));
        assert!(crate::repos::load(&file).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adding_a_non_repo_is_rejected() {
        let mut app = test_app();
        let dir = std::env::temp_dir();
        let response = call(
            &mut app,
            Method::RepoAdd(RepoAddParams {
                root: dir.join("herdr-definitely-missing").display().to_string(),
                ..RepoAddParams::default()
            }),
        );
        assert_eq!(response["error"]["code"], "repo_root_not_found");
        let relative = call(
            &mut app,
            Method::RepoAdd(RepoAddParams {
                root: "code/project".into(),
                ..RepoAddParams::default()
            }),
        );
        assert_eq!(relative["error"]["code"], "invalid_repo_root");
        assert!(app.state.repos.is_empty());
    }
}
