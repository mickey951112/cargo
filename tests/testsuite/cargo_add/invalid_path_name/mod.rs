use cargo_test_support::compare::assert_ui;
use cargo_test_support::prelude::*;
use cargo_test_support::Project;

use crate::cargo_add::init_registry;
use cargo_test_support::curr_dir;

#[cargo_test]
fn invalid_path_name() {
    init_registry();
    let project = Project::from_template(curr_dir!().join("in"));
    let project_root = project.root();
    let cwd = project_root.join("primary");

    snapbox::cmd::Command::cargo_ui()
        .arg("add")
        .arg_line("not-at-path --path ../dependency")
        .current_dir(&cwd)
        .assert()
        .code(101)
        .stdout_matches_path(curr_dir!().join("stdout.log"))
        .stderr_matches_path(curr_dir!().join("stderr.log"));

    assert_ui().subset_matches(curr_dir!().join("out"), &project_root);
}
