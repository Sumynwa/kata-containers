// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Cloud Hypervisor helper to boot a pod VM.

use anyhow::{Context, Result};
use slog::{debug};
use std::sync::Arc;
use kata_types::config::hypervisor::Hypervisor as HypervisorConfig;
use hypervisor::ch::CloudHypervisor;
use std::collections::HashMap;
use hypervisor::Hypervisor;
use super::TestVm;

// Helper fn to boot up a vm.
// A rough flow for booting a vm.
// - Launch cloud-hypervisor daemon
// - setup api client communication
// - start virtiofsd daemon
// - prepare vm info
// - boot vm using this info
pub(crate) async fn setup_test_vm(config: HypervisorConfig) -> Result<TestVm> {
    debug!(sl!(), "clh: booting up a test vm");
    
    let hypervisor = CloudHypervisor::new();
    hypervisor.set_hypervisor_config(config).await;

    // prepare vm
    // we do not pass any network namesapce since we dont want any
    let empty_anno_map: HashMap<String, String> = HashMap::new();
    hypervisor.prepare_vm("test-clh-vm", None, &empty_anno_map).await.context("prepare test vm")?;

    // start vm
    hypervisor.start_vm(10_000).await.context("start vm")?;

    let agent_socket_addr = hypervisor.get_agent_socket().await.context("get agent socket path")?;

    // return the vm structure
    Ok(TestVm{
        hypervisor_name: "cloud_hypervisor".to_string(),
        hypervisor_instance: Arc::new(hypervisor),
        socket_addr: agent_socket_addr,
        is_hybrid_vsock: true,
    })
}

pub(crate) async fn stop_test_vm(instance: Arc<dyn Hypervisor>) -> Result<()> {
    debug!(sl!(), "clh: stopping the test vm");

    instance.stop_vm().await.context("stop vm")?;

    Ok(())
}
