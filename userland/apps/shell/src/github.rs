//! GitHub on the handheld: the account its key belongs to (Settings › Keys ›
//! GitHub: a token with repository access), that account's repositories, and
//! the name and address commits are made with.
//!
//! The API is asked with curl over HTTPS, the token passed to curl in a file
//! only root can read, never on its command line. Git itself gets the token
//! from a credential helper (`/usr/libexec/catalyst/git-credential`, named in
//! `/etc/gitconfig` for https://github.com only) that reads the keys file:
//! never on a command line, never in a repository's own config, and for no
//! other server. Other HTTPS remotes work as git always does (public ones
//! need nothing).
//!
//! The simulator answers from a few made-up repositories unless told to ask
//! GitHub (`CATALYST_GITHUB_LIVE`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const API: &str = "https://api.github.com";

fn demo() -> bool {
    !cfg!(target_arch = "aarch64") && std::env::var_os("CATALYST_GITHUB_LIVE").is_none()
}

/// The token from Settings › Keys.
pub fn token() -> Option<String> {
    crate::keys::Keys::load().get("github").map(str::to_owned)
}

/// Who the key belongs to, as GitHub said when it was signed in with: the
/// login, the name shown, and the address commits carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub login: String,
    pub name: String,
    pub email: String,
}

fn account_path() -> PathBuf {
    crate::config_dir().join("github")
}

impl Account {
    /// `login=`, `name=`, `email=` lines, as the credential helper reads them.
    fn parse(text: &str) -> Option<Account> {
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(|v| v.trim().to_owned());
        let login = field("login").filter(|l| !l.is_empty())?;
        Some(Account { name: field("name").filter(|n| !n.is_empty()).unwrap_or_else(|| login.clone()), email: field("email").unwrap_or_default(), login })
    }

    fn text(&self) -> String {
        format!("login={}\nname={}\nemail={}\n", self.login, self.name, self.email)
    }
}

/// The account signed in, if any (and only while its key is still set).
pub fn account() -> Option<Account> {
    token()?;
    Account::parse(&std::fs::read_to_string(account_path()).ok()?)
}

fn save_account(account: Option<&Account>) {
    match account {
        Some(a) => {
            std::fs::create_dir_all(crate::config_dir()).ok();
            crate::sys::write_atomic(&account_path(), a.text().as_bytes()).ok();
        }
        None => {
            std::fs::remove_file(account_path()).ok();
        }
    }
}

/// GitHub's `/user`: the login, the name (or the login), and the address
/// commits are made with: the public one, else GitHub's private no-reply
/// address for the account (`<id>+<login>@users.noreply.github.com`).
pub fn parse_user(v: &Value) -> Option<Account> {
    let login = v["login"].as_str().filter(|l| !l.is_empty())?.to_owned();
    let name = v["name"].as_str().filter(|n| !n.trim().is_empty()).map_or_else(|| login.clone(), str::to_owned);
    let email = match (v["email"].as_str().filter(|e| e.contains('@')), v["id"].as_u64()) {
        (Some(e), _) => e.to_owned(),
        (None, Some(id)) => format!("{id}+{login}@users.noreply.github.com"),
        (None, None) => format!("{login}@users.noreply.github.com"),
    };
    Some(Account { login, name, email })
}

/// One of the account's repositories.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteRepo {
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub description: String,
    pub clone_url: String,
    /// When it last changed, as GitHub gives it (`2026-11-20T21:15:00Z`).
    pub updated: String,
    pub default_branch: String,
}

/// `/user/repos`: an array of repositories.
pub fn parse_repos(v: &Value) -> Vec<RemoteRepo> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let s = |k: &str| r[k].as_str().unwrap_or_default().to_owned();
            let clone_url = s("clone_url");
            (!clone_url.is_empty()).then(|| RemoteRepo {
                name: s("name"),
                full_name: s("full_name"),
                private: r["private"].as_bool().unwrap_or(false),
                description: s("description"),
                clone_url,
                updated: s("updated_at"),
                default_branch: s("default_branch"),
            })
        })
        .collect()
}

/// "2026-11-20T21:15:00Z" as "nov 20" (or the year, if not this one).
pub fn day(stamp: &str) -> String {
    let date = stamp.get(..10).unwrap_or_default();
    match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) if d.format("%Y").to_string() == chrono::Local::now().format("%Y").to_string() => d.format("%b %-d").to_string().to_lowercase(),
        Ok(d) => d.format("%b %Y").to_string().to_lowercase(),
        Err(_) => String::new(),
    }
}

/// Sign in with the key: who it belongs to, kept for commits and the
/// credential helper.
pub fn sign_in() -> Result<Account, String> {
    let account = parse_user(&get("/user")?).ok_or("GitHub did not say whose key it is")?;
    save_account(Some(&account));
    Ok(account)
}

/// Forget the account (the key itself stays in Settings › Keys).
pub fn sign_out() {
    save_account(None);
}

/// The account's repositories, most recently changed first: its own, and
/// those it works on with others.
pub fn repos() -> Result<Vec<RemoteRepo>, String> {
    Ok(parse_repos(&get("/user/repos?per_page=100&sort=updated&affiliation=owner,collaborator,organization_member")?))
}

/// The name and address a commit is made with: the account's, else the
/// handheld's own name.
pub fn identity() -> (String, String) {
    match account() {
        Some(a) => (a.name, a.email),
        None => (crate::device::name(), "handheld@catalyst.invalid".into()),
    }
}

/// One GET to GitHub's API, as JSON.
fn get(path: &str) -> Result<Value, String> {
    let token = token().ok_or("no GitHub key yet · add one in Settings › Keys")?;
    if demo() {
        return Ok(demo_answer(path));
    }
    // The token reaches curl in a file only root reads.
    let headers = format!("Authorization: Bearer {token}\nAccept: application/vnd.github+json\nX-GitHub-Api-Version: 2022-11-28\nUser-Agent: catalyst-linux\n");
    std::fs::create_dir_all(crate::config_dir()).ok();
    let file = crate::config_dir().join(".github-request");
    crate::sys::write_private(&file, headers.as_bytes()).map_err(|_| "could not prepare the request".to_owned())?;
    let out = Command::new("curl")
        .args(["-sS", "--max-time", "20", "--proto", "=https", "-w", "\n%{http_code}", "-H"])
        .arg(format!("@{}", file.display()))
        .arg(format!("{API}{path}"))
        .stdin(Stdio::null())
        .output();
    std::fs::remove_file(&file).ok();
    let out = out.map_err(|_| "curl could not start".to_owned())?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) => "no internet: github.com did not resolve".into(),
            Some(7) | Some(28) => "could not reach GitHub".into(),
            Some(35) | Some(60) => "the secure link to GitHub failed · is the clock right?".into(),
            _ => "could not ask GitHub".into(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    match code.trim().parse::<u16>().unwrap_or(0) {
        200 => serde_json::from_str(body).map_err(|_| "GitHub's answer could not be read".to_owned()),
        401 => Err("GitHub refused the key · check it in Settings › Keys".into()),
        403 | 429 => Err("GitHub says too many requests · try again later".into()),
        code => Err(format!("GitHub answered {code}")),
    }
}

// ------------------------------------------------------------------ demo ---

fn demo_answer(path: &str) -> Value {
    if path.starts_with("/user/repos") {
        let repo = |name: &str, private: bool, about: &str, updated: &str| {
            json!({"name": name, "full_name": format!("team5805/{name}"), "private": private, "description": about,
                   "clone_url": format!("https://github.com/team5805/{name}.git"), "updated_at": updated, "default_branch": "main"})
        };
        json!([
            repo("robot-2027", true, "Competition robot code", "2026-11-20T21:15:00Z"),
            repo("pit-checklist", false, "What gets checked between matches", "2026-11-18T16:02:00Z"),
            repo("autos", true, "Autonomous routines and paths", "2026-11-12T09:40:00Z"),
            repo("scouting-app", false, "Match scouting on phones", "2026-10-30T18:22:00Z"),
            repo("vision", true, "AprilTag pose estimation", "2026-09-14T11:05:00Z"),
        ])
    } else {
        json!({"login": "team5805", "id": 1234567, "name": "Team 5805", "email": null})
    }
}

/// A repository with some history to look at, for the simulator: commits on
/// main, a branch that was merged back, and one still open.
pub fn make_demo_repo(dir: &Path) -> Result<(), String> {
    if dir.join(".git").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let git = |args: &[&str]| -> Result<(), String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Team 5805", "-c", "user.email=team@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .env("GIT_COMMITTER_DATE", "2026-11-20T21:00:00")
            .env("GIT_AUTHOR_DATE", "2026-11-20T21:00:00")
            .output()
            .map_err(|_| "git is not installed".to_owned())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    };
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).map_err(|e| e.to_string());
    git(&["init", "-q", "-b", "main"])?;
    write("README.md", "# robot-2027\n\nCompetition robot code.\n")?;
    write("gradlew", "#!/bin/sh\necho deploying\n")?;
    git(&["add", "."])?;
    git(&["commit", "-q", "-m", "Start the 2027 robot"])?;
    write("Robot.java", "public class Robot {\n    // drive\n}\n")?;
    git(&["add", "."])?;
    git(&["commit", "-q", "-m", "Drive base with swerve modules"])?;
    git(&["switch", "-q", "-c", "intake"])?;
    write("Intake.java", "public class Intake {\n    double currentLimit = 40;\n}\n")?;
    git(&["add", "."])?;
    git(&["commit", "-q", "-m", "Intake with a 40 A current limit"])?;
    git(&["switch", "-q", "main"])?;
    write("Robot.java", "public class Robot {\n    // drive\n    // auto: two notes\n}\n")?;
    git(&["commit", "-q", "-am", "Two-note auto"])?;
    git(&["merge", "-q", "--no-ff", "intake", "-m", "Merge the intake"])?;
    write("Intake.java", "public class Intake {\n    double currentLimit = 35;\n}\n")?;
    git(&["commit", "-q", "-am", "Lower the intake's limit to 35 A after Q14"])?;
    git(&["switch", "-q", "-c", "shooter-tuning"])?;
    write("Shooter.java", "public class Shooter {\n    double rpm = 4200;\n}\n")?;
    git(&["add", "."])?;
    git(&["commit", "-q", "-m", "Shooter at 4200 rpm"])?;
    git(&["switch", "-q", "main"])?;
    write("Robot.java", "public class Robot {\n    // drive\n    // auto: three notes\n}\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whose_key_it_is_and_the_address_commits_carry() {
        // As GitHub answers /user (docs.github.com, "Get the authenticated user").
        let public = json!({"login": "octocat", "id": 1, "name": "The Octocat", "email": "octocat@github.com"});
        assert_eq!(parse_user(&public), Some(Account { login: "octocat".into(), name: "The Octocat".into(), email: "octocat@github.com".into() }));
        // A private address: GitHub's no-reply one for the account.
        let private = json!({"login": "team5805", "id": 1234567, "name": null, "email": null});
        let a = parse_user(&private).unwrap();
        assert_eq!((a.name.as_str(), a.email.as_str()), ("team5805", "1234567+team5805@users.noreply.github.com"));
        assert_eq!(parse_user(&json!({"message": "Bad credentials"})), None);
        // Kept as the credential helper reads it.
        assert_eq!(Account::parse(&a.text()), Some(a));
    }

    #[test]
    fn the_accounts_repositories_are_read() {
        // The fields as GitHub's /user/repos gives them (checked against a real answer).
        let v = json!([
            {"name": "robot", "full_name": "team/robot", "private": true, "description": null, "clone_url": "https://github.com/team/robot.git", "updated_at": "2026-11-20T21:15:00Z", "default_branch": "main", "ssh_url": "git@github.com:team/robot.git"},
            {"name": "broken"}
        ]);
        let repos = parse_repos(&v);
        assert_eq!(repos.len(), 1, "one without an address is passed over");
        assert_eq!((repos[0].full_name.as_str(), repos[0].private, repos[0].description.as_str()), ("team/robot", true, ""));
        assert_eq!(parse_repos(&json!({"message": "Bad credentials"})), []);
        assert!(day("2001-02-03T04:05:06Z").ends_with("2001"));
        assert_eq!(day("not a date"), "");
    }
}
