# Tutorial 05 - Safe Globals

## A slightly longer tl;dr

When we introduced the globally usable `print!` macros in [tutorial 03], we
cheated a bit. Calling `core::fmt`'s `write_fmt()` function, which takes an
`&mut self`, was only working because on each call, a new instance of
`QEMUOutput` was created.

If we would want to preserve some state, e.g. statistics about the number of
characters written, we need to make a single global instance of `QEMUOutput` (in
Rust, using the `static` keyword).

A `static QEMU_OUTPUT`, however, would not allow to call functions taking `&mut
self`. For that, we would need a `static mut`, but calling functions that mutate
state on `static mut`s is unsafe. The Rust compiler's reasoning for this is that
it can then not prevent anymore that multiple cores/threads are mutating the
data concurrently (it is a global, so everyone can reference it from anywhere.
The borrow checker can't help here).

The solution to this problem is to wrap the global into a synchronization
primitive. In our case, a variant of a *MUTual EXclusion* primivite. `Mutex` is
introduced as a trait in `interfaces.rs`, and implemented by the name of
`NullLock` in `sync.rs` in the `bsp` folder. For teaching purposes, to make the
code lean, it leaves out the actual platform-specific logic for protection
against concurrent access, since we don't need it as long as the kernel only
exeuts on a single core with interrupts disabled.

Instead, it focuses on showcasing the core concept of [interior mutability].
Make sure to read up on it. I also recommend to read this article about an
[accurate mental model for Rust's reference types].

If you want to compare the `NullLock` to some real-world implementations, you
can check out implemntations in the [spin crate] or the [parking lot crate].

[tutorial 03]: ../03_hacky_hello_world
[interior mutability]: https://doc.rust-lang.org/std/cell/index.html
[accurate mental model for Rust's reference types]: https://docs.rs/dtolnay/0.0.6/dtolnay/macro._02__reference_types.html
[spin crate]: https://github.com/mvdnes/spin-rs
[parking lot crate]: https://github.com/Amanieu/parking_lot

## Diff to previous
```diff

diff -uNr 04_zero_overhead_abstraction/src/bsp/rpi3/sync.rs 05_safe_globals/src/bsp/rpi3/sync.rs
--- 04_zero_overhead_abstraction/src/bsp/rpi3/sync.rs
+++ 05_safe_globals/src/bsp/rpi3/sync.rs
@@ -0,0 +1,47 @@
+// SPDX-License-Identifier: MIT
+//
+// Copyright (c) 2018-2019 Andre Richter <andre.o.richter@gmail.com>
+
+//! Board-specific synchronization primitives.
+
+use crate::interface;
+use core::cell::UnsafeCell;
+
+/// A pseudo-lock for teaching purposes.
+///
+/// Used to introduce [interior mutability].
+///
+/// In contrast to a real Mutex implementation, does not protect against
+/// concurrent access to the contained data. This part is preserved for later
+/// lessons.
+///
+/// The lock will only be used as long as it is safe to do so, i.e. as long as
+/// the kernel is executing single-threaded, aka only running on a single core
+/// with interrupts disabled.
+///
+/// [interior mutability]: https://doc.rust-lang.org/std/cell/index.html
+pub struct NullLock<T: ?Sized> {
+    data: UnsafeCell<T>,
+}
+
+unsafe impl<T: ?Sized + Send> Send for NullLock<T> {}
+unsafe impl<T: ?Sized + Send> Sync for NullLock<T> {}
+
+impl<T> NullLock<T> {
+    pub const fn new(data: T) -> NullLock<T> {
+        NullLock {
+            data: UnsafeCell::new(data),
+        }
+    }
+}
+
+impl<T> interface::sync::Mutex for &NullLock<T> {
+    type Data = T;
+
+    fn lock<R>(&mut self, f: impl FnOnce(&mut Self::Data) -> R) -> R {
+        // In a real lock, there would be code encapsulating this line that
+        // ensures that this mutable reference will ever only be given out once
+        // at a time.
+        f(unsafe { &mut *self.data.get() })
+    }
+}

diff -uNr 04_zero_overhead_abstraction/src/bsp/rpi3.rs 05_safe_globals/src/bsp/rpi3.rs
--- 04_zero_overhead_abstraction/src/bsp/rpi3.rs
+++ 05_safe_globals/src/bsp/rpi3.rs
@@ -5,10 +5,12 @@
 //! Board Support Package for the Raspberry Pi 3.

 mod panic_wait;
+mod sync;

 use crate::interface;
 use core::fmt;
 use cortex_a::{asm, regs::*};
+use sync::NullLock;

 /// The entry of the `kernel` binary.
 ///
@@ -38,28 +40,100 @@
 }

 /// A mystical, magical device for generating QEMU output out of the void.
-struct QEMUOutput;
+///
+/// The mutex protected part.
+struct QEMUOutputInner {
+    chars_written: usize,
+}

-/// Implementing `console::Write` enables usage of the `format_args!` macros,
+impl QEMUOutputInner {
+    const fn new() -> QEMUOutputInner {
+        QEMUOutputInner { chars_written: 0 }
+    }
+
+    /// Send a character.
+    fn write_char(&mut self, c: char) {
+        unsafe {
+            core::ptr::write_volatile(0x3F21_5040 as *mut u8, c as u8);
+        }
+    }
+}
+
+/// Implementing `core::fmt::Write` enables usage of the `format_args!` macros,
 /// which in turn are used to implement the `kernel`'s `print!` and `println!`
-/// macros.
+/// macros. By implementing `write_str()`, we get `write_fmt()` automatically.
+///
+/// The function takes an `&mut self`, so it must be implemented for the inner
+/// struct.
 ///
 /// See [`src/print.rs`].
 ///
 /// [`src/print.rs`]: ../../print/index.html
-impl interface::console::Write for QEMUOutput {
+impl fmt::Write for QEMUOutputInner {
     fn write_str(&mut self, s: &str) -> fmt::Result {
         for c in s.chars() {
-            unsafe {
-                core::ptr::write_volatile(0x3F21_5040 as *mut u8, c as u8);
+            // Convert newline to carrige return + newline.
+            if c == '
' {
+                self.write_char('
             }
+
+            self.write_char(c);
         }

+        self.chars_written += s.len();
+
         Ok(())
     }
 }

 ////////////////////////////////////////////////////////////////////////////////
+// OS interface implementations
+////////////////////////////////////////////////////////////////////////////////
+
+/// The main struct.
+pub struct QEMUOutput {
+    inner: NullLock<QEMUOutputInner>,
+}
+
+impl QEMUOutput {
+    pub const fn new() -> QEMUOutput {
+        QEMUOutput {
+            inner: NullLock::new(QEMUOutputInner::new()),
+        }
+    }
+}
+
+/// Passthrough of `args` to the `core::fmt::Write` implementation, but guarded
+/// by a Mutex to serialize access.
+impl interface::console::Write for QEMUOutput {
+    fn write_fmt(&self, args: core::fmt::Arguments) -> fmt::Result {
+        use interface::sync::Mutex;
+
+        // Fully qualified syntax for the call to
+        // `core::fmt::Write::write:fmt()` to increase readability.
+        let mut r = &self.inner;
+        r.lock(|i| fmt::Write::write_fmt(i, args))
+    }
+}
+
+impl interface::console::Read for QEMUOutput {}
+
+impl interface::console::Statistics for QEMUOutput {
+    fn chars_written(&self) -> usize {
+        use interface::sync::Mutex;
+
+        let mut r = &self.inner;
+        r.lock(|i| i.chars_written)
+    }
+}
+
+////////////////////////////////////////////////////////////////////////////////
+// Global instances
+////////////////////////////////////////////////////////////////////////////////
+
+static QEMU_OUTPUT: QEMUOutput = QEMUOutput::new();
+
+////////////////////////////////////////////////////////////////////////////////
 // Implementation of the kernel's BSP calls
 ////////////////////////////////////////////////////////////////////////////////

@@ -70,7 +144,7 @@
     }
 }

-/// Returns a ready-to-use `console::Write` implementation.
-pub fn console() -> impl interface::console::Write {
-    QEMUOutput {}
+/// Return a reference to a `console::All` implementation.
+pub fn console() -> &'static impl interface::console::All {
+    &QEMU_OUTPUT
 }

diff -uNr 04_zero_overhead_abstraction/src/interface.rs 05_safe_globals/src/interface.rs
--- 04_zero_overhead_abstraction/src/interface.rs
+++ 05_safe_globals/src/interface.rs
@@ -20,17 +20,68 @@

 /// System console operations.
 pub mod console {
+    use core::fmt;
+
     /// Console write functions.
-    ///
-    /// `core::fmt::Write` is exactly what we need for now. Re-export it here
-    /// because implementing `console::Write` gives a better hint to the reader
-    /// about the intention.
-    pub use core::fmt::Write;
+    pub trait Write {
+        fn write_fmt(&self, args: fmt::Arguments) -> fmt::Result;
+    }

     /// Console read functions.
     pub trait Read {
-        fn read_char(&mut self) -> char {
+        fn read_char(&self) -> char {
             ' '
         }
     }
+
+    /// Console statistics.
+    pub trait Statistics {
+        /// Return the number of characters written.
+        fn chars_written(&self) -> usize {
+            0
+        }
+
+        /// Return the number of characters read.
+        fn chars_read(&self) -> usize {
+            0
+        }
+    }
+
+    /// Trait alias for a full-fledged console.
+    pub trait All = Write + Read + Statistics;
+}
+
+/// Synchronization primitives.
+pub mod sync {
+    /// Any object implementing this trait guarantees exclusive access to the
+    /// data contained within the mutex for the duration of the lock.
+    ///
+    /// The trait follows the [Rust embedded WG's
+    /// proposal](https://github.com/korken89/wg/blob/master/rfcs/0377-mutex-trait.md)
+    /// and therefore provides some goodness such as [deadlock
+    /// prevention](https://github.com/korken89/wg/blob/master/rfcs/0377-mutex-trait.md#design-decisions-and-compatibility).
+    ///
+    /// # Example
+    ///
+    /// Since the lock function takes an `&mut self` to enable
+    /// deadlock-prevention, the trait is best implemented **for a reference to
+    /// a container struct**, and has a usage pattern that might feel strange at
+    /// first:
+    ///
+    /// ```
+    /// static MUT: Mutex<RefCell<i32>> = Mutex::new(RefCell::new(0));
+    ///
+    /// fn foo() {
+    ///     let mut r = &MUT; // Note that r is mutable
+    ///     r.lock(|data| *data += 1);
+    /// }
+    /// ```
+    pub trait Mutex {
+        /// Type of data encapsulated by the mutex.
+        type Data;
+
+        /// Creates a critical section and grants temporary mutable access to
+        /// the encapsulated data.
+        fn lock<R>(&mut self, f: impl FnOnce(&mut Self::Data) -> R) -> R;
+    }
 }

diff -uNr 04_zero_overhead_abstraction/src/main.rs 05_safe_globals/src/main.rs
--- 04_zero_overhead_abstraction/src/main.rs
+++ 05_safe_globals/src/main.rs
@@ -15,6 +15,7 @@

 #![feature(format_args_nl)]
 #![feature(panic_info_message)]
+#![feature(trait_alias)]
 #![no_main]
 #![no_std]

@@ -31,8 +32,12 @@

 /// Entrypoint of the `kernel`.
 fn kernel_entry() -> ! {
+    use interface::console::Statistics;
+
     println!("[0] Hello from pure Rust!");

-    println!("[1] Stopping here.");
+    println!("[1] Chars written: {}", bsp::console().chars_written());
+
+    println!("[2] Stopping here.");
     bsp::wait_forever()
 }
```