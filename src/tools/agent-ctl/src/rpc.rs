// Copyright (c) 2020 Intel Corporation
//
// SPDX-License-Identifier: Apache-2.0
//

// Description: ttRPC logic entry point

use anyhow::{anyhow, Result};
use slog::{o, Logger};

use crate::client::client;
use crate::types::Config;
use crate::vm;
use slog::debug;

pub fn run(logger: &Logger, cfg: &mut Config, commands: Vec<&str>) -> Result<()> {
    // Maintain the global logger for the duration of the ttRPC comms
    let _guard = slog_scope::set_global_logger(logger.new(o!("subsystem" => "rpc")));

    // Booting a test pod vm
    let test_vm_instance = vm::boot_test_vm()?;
    debug!(sl!(), "test vm booted for hypervisor: {:?}", test_vm_instance.hypervisor_name);

    // Check if we have a socket address.
    if cfg.server_address.is_empty() && test_vm_instance.socket_addr.is_empty() {
        debug!(sl!(), "failed to get valid socket address, cannot connect to agent");
        return Err(anyhow!("Failed to get agent socket address"));
    }

    // override the address here
    if !test_vm_instance.socket_addr.is_empty() {
        let addr_fields: Vec<&str> = test_vm_instance.socket_addr.split("://").collect();
        cfg.server_address = format!("{}://{}", "unix", addr_fields[1].to_string());
        cfg.hybrid_vsock = test_vm_instance.is_hybrid_vsock;
    }

    match client(cfg, commands) {
        Ok(_) => {
		debug!(sl!(), "Shutting down vm");
		vm::stop_test_vm(test_vm_instance.hypervisor_instance.clone())
	}
        Err(e) => {
		debug!(sl!(), "Command failed: {}", e);
		debug!(sl!(), "Shutting down vm");
		vm::stop_test_vm(test_vm_instance.hypervisor_instance.clone())
	}
    }
}
