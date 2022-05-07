use cargo_test_support::compare::assert;
use cargo_test_support::prelude::*;
use cargo_test_support::Project;

use crate::init::mercurial_available;
use cargo_test_support::curr_dir;

#[cargo_test]
fn simple_hg() {
    if !mercurial_available() {
        return;
    }

    let project = Project::from_template(curr_dir!().join("in"));
    let project_root = &project.root();

    snapbox::cmd::Command::cargo()
        .arg_line("init --lib --vcs hg")
        .current_dir(project_root)
        .assert()
        .success()
        .stdout_matches_path(curr_dir!().join("stdout.log"))
        .stderr_matches_path(curr_dir!().join("stderr.log"));

    assert().subset_matches(curr_dir!().join("out"), project_root);
    assert!(!project_root.join(".git").is_dir());
}
