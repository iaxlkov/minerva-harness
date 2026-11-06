#![cfg(target_os = "linux")]

use minerva_execution::{docker::*, *};
use std::{collections::BTreeMap, path::PathBuf};
use uuid::Uuid;
/// Explicit destructive-test opt-in: creates quota-bounded UUID subvolumes and runsc containers.
/// It never reconfigures Docker, networks, runtimes, or the Btrfs filesystem.
#[test]
#[ignore = "requires provisioned Linux Docker/runsc+Btrfs and MINERVA_RUNSC_CONFIG; not sandbox acceptance on Windows"]
fn real_linux_runsc_isolates_mounts_network_and_orphan_processes() {
    let config: DockerConfig = serde_json::from_slice(
        &std::fs::read(
            std::env::var("MINERVA_RUNSC_CONFIG").expect("operator test config required"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(std::env::var("MINERVA_RUNSC_TEST_OPT_IN").as_deref() == Ok("yes"));
    let mut driver = DockerDriver::new(config.clone()).unwrap();
    driver.ready().unwrap();
    let a = Assignment {
        tenant_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        assignment_id: Uuid::new_v4(),
        worker_id: Uuid::new_v4(),
        generation: 1,
    };
    let limits = Limits {
        memory_mib: 128,
        cpu_milli: 250,
        pids: 32,
        disk_mib: 128,
    };
    let mounts = driver.prepare(&a, &limits).unwrap();
    std::fs::write(mounts.data.join("private-sentinel"), "private").unwrap();
    let other = Assignment {
        workspace_id: Uuid::new_v4(),
        ..a.clone()
    };
    let other_mounts = driver.prepare(&other, &limits).unwrap();
    std::fs::write(other_mounts.workspace.join("neighbor-sentinel"), "neighbor").unwrap();
    let launch = Launch {
        assignment: a.clone(),
        limits,
        mounts,
    };
    let command = |script: &str, timeout| Execution {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        env: BTreeMap::new(),
        timeout_ms: timeout,
    };
    let result=driver.execute(&launch,&command("test ! -e /data/private-sentinel && test ! -e /config && test ! -e /var/run/docker.sock && test ! -e /workspace/neighbor-sentinel && ln -s /data/private-sentinel /workspace/escape && test ! -e /workspace/escape && test ! -e /sys/class/net/eth0 && printf isolated",5000)).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("isolated"));
    let result = driver
        .execute(
            &launch,
            &command("(sleep 1; printf escaped > /workspace/orphan) & wait", 50),
        )
        .unwrap();
    assert!(result.timed_out);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!launch.mounts.workspace.join("orphan").exists());
    assert_eq!(
        std::fs::read_to_string(other_mounts.workspace.join("neighbor-sentinel")).unwrap(),
        "neighbor"
    );
    driver.stop_workspace(a.tenant_id, a.workspace_id).unwrap();
    driver
        .stop_workspace(other.tenant_id, other.workspace_id)
        .unwrap();
    // Leave test-owned UUID subvolumes for operator inspection; README documents precise cleanup.
    let _: PathBuf = config.root;
}
