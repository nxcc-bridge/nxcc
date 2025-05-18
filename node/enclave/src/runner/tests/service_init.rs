use std::sync::Arc;

use super::common::*;

#[tokio::test]
async fn test_new_runner_service() {
    let (secrets, runner_service, _) = setup();
    assert!(runner_service.vms.read().await.is_empty());
    assert!(runner_service.worker_map.read().await.is_empty());
    // Check if the secrets Arc points to the same allocation
    assert!(Arc::ptr_eq(&runner_service.secrets, &secrets));
}

// Note: We don't test the real attach_vm due to network/TLS complexity.
// We test the state changes via manual insertion and detach_vm.

#[tokio::test]
async fn test_detach_vm_exists() {
    let (_secrets, runner_service, mock_client) = setup();
    let vm_id = "vm-1";
    let worker_id_1 = "worker-on-vm1-1";
    let worker_id_2 = "worker-on-vm1-2";
    let worker_id_other = "worker-on-vm2";

    attach_mock_vm(&runner_service, vm_id, mock_client).await;
    add_worker_mapping(&runner_service, worker_id_1, vm_id).await;
    add_worker_mapping(&runner_service, worker_id_2, vm_id).await;
    add_worker_mapping(&runner_service, worker_id_other, "vm-2").await; // Belongs to another VM

    assert!(runner_service.vms.read().await.contains_key(vm_id));
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_1)
    );
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_2)
    );
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_other)
    );

    let result = runner_service.detach_vm(vm_id.to_string()).await;
    result.unwrap(); // Expect Ok

    assert!(!runner_service.vms.read().await.contains_key(vm_id));
    // Check workers associated with vm_id are removed
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_1)
    );
    assert!(
        !runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_2)
    );
    // Check worker on other VM remains
    assert!(
        runner_service
            .worker_map
            .read()
            .await
            .contains_key(worker_id_other)
    );
}

#[tokio::test]
async fn test_detach_vm_not_exists() {
    let (_secrets, runner_service, _mock_client) = setup();
    let vm_id = "vm-nonexistent";

    assert!(!runner_service.vms.read().await.contains_key(vm_id));

    // Detaching a non-existent VM should be Ok (idempotent)
    let result = runner_service.detach_vm(vm_id.to_string()).await;
    result.unwrap();

    assert!(!runner_service.vms.read().await.contains_key(vm_id));
    assert!(runner_service.worker_map.read().await.is_empty());
}
