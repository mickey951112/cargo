use cargo_test_support::compare::assert;
use cargo_test_support::prelude::*;
use cargo_test_support::Project;

use crate::cargo_add::init_registry;
use cargo_test_support::curr_dir;

#[cargo_test]
fn namever() {
    init_registry();
    let project = Project::from_template(curr_dir!().join("in"));
    let project_root = project.root();
    let cwd = &project_root;

    snapbox::cmd::Command::cargo()
        .arg("add")
        .arg_line("my-package1@>=0.1.1 my-package2@0.2.3 my-package")
        .current_dir(cwd)
        .assert()
        .success()
        .stdout_matches_path(curr_dir!().join("stdout.log"))
        .stderr_matches_path(curr_dir!().join("stderr.log"));

    assert().subset_matches(curr_dir!().join("out"), &project_root);
}
