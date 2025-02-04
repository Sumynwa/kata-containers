// Copyright (c) 2020 Intel Corporation
//
// SPDX-License-Identifier: Apache-2.0
//

// Description: ttRPC logic entry point

use anyhow::Result;
use slog::{o, Logger};

use crate::client::client;
use crate::types::Config;
use crate::vm;
use slog::debug;

pub fn run(logger: &Logger, cfg: &Config, commands: Vec<&str>) -> Result<()> {
    // Maintain the global logger for the duration of the ttRPC comms
    let _guard = slog_scope::set_global_logger(logger.new(o!("subsystem" => "rpc")));

    // Booting a test pod vm
    let test_vm_instance = vm::boot_test_vm()?;
    debug!(sl!(), "test vm booted for hypervisor: {:?}", test_vm_instance.hypervisor_name);

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
