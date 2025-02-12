// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Boot UVM for testing container storages/volumes.

use anyhow::{anyhow, Context, Result};
use slog::{debug, warn};
use std::sync::Arc;
use hypervisor::Hypervisor;
use hypervisor::device::device_manager::DeviceManager;
use kata_types::config::TomlConfig;
use tokio::sync::RwLock;

mod clh;
mod qemu;

pub struct TestVm {
    pub hypervisor_name: String,
    pub hypervisor_instance: Arc<dyn Hypervisor>,
    #[allow(dead_code)]
    pub device_manager: Arc<RwLock<DeviceManager>>,
    pub socket_addr: String,
    pub is_hybrid_vsock: bool,
}

// Helper function to parse a configuration file.
fn load_config(config_file: &str) -> Result<TomlConfig> {
    debug!(sl!(), "test_vm_boot: Load kata configuration file");

    let (mut toml_config, _) = TomlConfig::load_from_file(config_file).context("test_vm_boot:Failed to load kata config file")?;

    // Update the agent kernel params in hypervisor config
    update_agent_kernel_params(&mut toml_config)?;

    // validate configuration and return the error
    toml_config.validate()?;

    debug!(sl!(), "get config content {:?}", &toml_config);
    Ok(toml_config)
}

pub fn to_kernel_string(key:String, val: String) -> Result<String> {
    if key.is_empty() && val.is_empty() {
        Err(anyhow!("Empty key and value"))
    } else if key.is_empty() {
        Err(anyhow!("Empty key"))
    } else if val.is_empty() {
        Ok(key.to_string())
    } else {
        Ok(format!("{}{}{}", key, "=", val))
    }
}

fn update_agent_kernel_params(config: &mut TomlConfig) -> Result<()> {
    let mut params = vec![];
    if let Ok(kv) = config.get_agent_kernel_params() {
        for (k, v) in kv.into_iter() {
            if let Ok(s) = to_kernel_string(k.to_owned(), v.to_owned()) {
                params.push(s);
            }
        }
        if let Some(h) = config.hypervisor.get_mut(&config.runtime.hypervisor_name) {
            h.boot_info.add_kernel_params(params);
        }
    }
    Ok(())
}

// virtiofsd - need to start this as well???
// Look into this for CLH
// crates/runtimes/virt_container/src/lib.rs: new_hypervisor()::CloudHypervisor::new()
//                                            set_config()
// crates/runtimes/virt_container/src/sandbox.rs:: start()
// Not sure but need to see what all happens with the hypervisor config we have from the toml
// Can we simply use that config and give it to CLH API client to create a vm??
// prepare_vm:
// - set ns to none
// - dont handle confidential guests
// - look at setting the run paths on host
//         // run_dir and vm_path are the same (shared)
//        self.run_dir = get_sandbox_path(&self.id);
//        self.vm_path = self.run_dir.to_string();
//
// Using the runtime-rs hypervisor crate.
// 1. Launch the clh process: Need to initialize CloudHypervisor::CloudHypervisorInner
// 2. Look into CloudHypervisorInner: start_hypervisor (are these pub functions??)
// 3. prepare_hypervisor
//    launch_hypervisor
//    prepare_vm
//    start_vm

// Helper method to boot a test pod VM
pub fn boot_test_vm(hypervisor_name: String) -> Result<TestVm> {
    debug!(sl!(), "boot_test_vm: Booting up a test pod vm with {:?}", hypervisor_name);

    // create a new hypervisor instance
    match hypervisor_name.as_str() {
        clh::CLH_HYP => {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(clh::setup_test_vm())
                .context("setting up test vm using Cloud Hypervisor");

        }
        qemu::QEMU_HYP => {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(qemu::setup_test_vm())
                .context("setting up test vm using Qemu");
        }
        _ => {
            warn!(sl!(), "boot_test_vm: Unsupported hypervisor : {:?}", hypervisor_name);
            return Err(anyhow!(
                    "boot_test_vm: Unsupported hypervisor name"
            ));
        }
    }
}

// Helper method to shutdown a test pod VM
pub fn stop_test_vm(instance: Arc<dyn Hypervisor>) -> Result<()> {
    debug!(sl!(), "stop_test_vm: stopping booted vm");

    let _ = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(clh::stop_test_vm(instance))
        .context("stop booted test vm")?;

    Ok(())
}
