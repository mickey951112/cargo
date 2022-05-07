use cargo_test_support::compare::assert;
use cargo_test_support::prelude::*;
use cargo_test_support::Project;

use crate::cargo_add::init_registry;
use cargo_test_support::curr_dir;

#[cargo_test]
fn path_normalized_name() {
    init_registry();
    let project = Project::from_template(curr_dir!().join("in"));
    let project_root = project.root();
    let cwd = project_root.join("primary");

    snapbox::cmd::Command::cargo()
        .arg("add")
        .arg_line("cargo_list_test_fixture_dependency --path ../dependency")
        .current_dir(&cwd)
        .assert()
        .failure() // Fuzzy searching for paths isn't supported at this time
        .stdout_matches_path(curr_dir!().join("stdout.log"))
        .stderr_matches_path(curr_dir!().join("stderr.log"));

    assert().subset_matches(curr_dir!().join("out"), &project_root);
}
