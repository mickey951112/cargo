use cargo_test_support::compare::assert_ui;
use cargo_test_support::curr_dir;
use cargo_test_support::CargoCommand;
use cargo_test_support::Project;
use cargo_test_support::TestEnv;

use crate::cargo_remove::init_registry;

#[cargo_test]
fn case() {
    init_registry();
    let project = Project::from_template(curr_dir!().join("in"));
    let project_root = project.root();
    let cwd = &project_root;

    // run the metadata command to populate the cache
    let cargo = std::env::var_os("CARGO").unwrap();
    snapbox::cmd::Command::new(cargo)
        .test_env()
        .arg("metadata")
        .current_dir(cwd)
        .assert()
        .success();

    snapbox::cmd::Command::cargo_ui()
        .arg("remove")
        .args(["docopt", "--offline"])
        .current_dir(cwd)
        .assert()
        .success()
        .stdout_matches_path(curr_dir!().join("stdout.log"))
        .stderr_matches_path(curr_dir!().join("stderr.log"));

    assert_ui().subset_matches(curr_dir!().join("out"), &project_root);
}
