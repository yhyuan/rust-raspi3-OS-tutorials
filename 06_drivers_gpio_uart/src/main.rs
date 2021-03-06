// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Copyright (c) 2018-2019 Andre Richter <andre.o.richter@gmail.com>

// Rust embedded logo for `make doc`.
#![doc(html_logo_url = "https://git.io/JeGIp")]

//! The `kernel`
//!
//! The `kernel` is composed by glueing together code from
//!
//!   - [Hardware-specific Board Support Packages] (`BSPs`).
//!   - [Architecture-specific code].
//!   - HW- and architecture-agnostic `kernel` code.
//!
//! using the [`kernel::interface`] traits.
//!
//! [Hardware-specific Board Support Packages]: bsp/index.html
//! [Architecture-specific code]: arch/index.html
//! [`kernel::interface`]: interface/index.html

#![feature(format_args_nl)]
#![feature(panic_info_message)]
#![feature(trait_alias)]
#![no_main]
#![no_std]

// Conditionally includes the selected `architecture` code, which provides the `_start()` function,
// the first function to run.
mod arch;

// `_start()` then calls `runtime_init()`, which on completion, jumps to `kernel_init()`.
mod runtime_init;

// Conditionally includes the selected `BSP` code.
mod bsp;

mod interface;
mod panic_wait;
mod print;

/// Early init code.
///
/// Concerned with with initializing `BSP` and `arch` parts.
///
/// # Safety
///
/// - Only a single core must be active and running this function.
/// - The init calls in this function must appear in the correct order.
unsafe fn kernel_init() -> ! {
    for i in bsp::device_drivers().iter() {
        if let Err(()) = i.init() {
            panic!("Error loading driver: {}", i.compatible())
        }
    }
    bsp::post_driver_init();
    // println! is usable from here on.

    // Transition from unsafe to safe.
    kernel_main()
}

/// The main function running after the early init.
fn kernel_main() -> ! {
    use interface::console::All;

    // UART should be functional now. Wait for user to hit Enter.
    loop {
        if bsp::console().read_char() == '\n' {
            break;
        }
    }

    println!("[0] Booting on: {}", bsp::board_name());

    println!("[1] Drivers loaded:");
    for (i, driver) in bsp::device_drivers().iter().enumerate() {
        println!("      {}. {}", i + 1, driver.compatible());
    }

    println!("[2] Chars written: {}", bsp::console().chars_written());
    println!("[3] Echoing input now");

    loop {
        let c = bsp::console().read_char();
        bsp::console().write_char(c);
    }
}
