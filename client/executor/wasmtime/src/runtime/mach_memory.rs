// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Defines a custom memory allocator for allocating host memory on the mach kernel.
//! This is needed in order to support purgable memory on macOS.

use mach::{
    kern_return::KERN_SUCCESS,
    traps::mach_task_self,
	port::mach_port_t,
    vm::{mach_vm_allocate, mach_vm_protect},
    vm_types::{mach_vm_address_t, mach_vm_size_t},
    vm_prot::{vm_prot_t, VM_PROT_NONE, VM_PROT_DEFAULT},
};
use wasmtime::{MemoryCreator, LinearMemory, MemoryType};
use std::sync::Mutex;

const WASM_PAGE_SHIFT: u64 = 16;

pub struct MachAllocator {
	task: mach_port_t,
}

pub struct MachMemory {
	/// The virtual address of the mapping.
	address: mach_vm_address_t,
	/// The size of the mapping created in bytes.
	///
	/// If this memory is grown beyond the virtual size we need to allocate a new
	/// a new mapping and copy over.
	mapped_bytes: u64,
	/// Size of the guard pages in bytes.
	guard_bytes: u64,
	/// The currently accesible number of wasm pages.
	///
	/// Starting with wasmtime 0.28 we can remove the mutex as `LinearMeory::grow` takes
	/// an exclusive reference to this struct.
	wasm_pages: Mutex<u32>,
	/// The maximum number was wasm pages this memory is allowed to be growed to.
	wasm_pages_max: Option<u32>,
}

impl MachAllocator {
	pub fn new() -> Result<Self, String> {
		// SAFETY:
		// There are no preconditions. It is unsafe only because it is a C-API.
		let task = unsafe { mach_task_self() };
		Ok(Self {
			task,
		})
	}
}

unsafe impl MemoryCreator for MachAllocator {
    fn new_memory(
        &self,
        ty: MemoryType,
        reserved_size_in_bytes: Option<u64>,
        guard_size_in_bytes: u64
    ) -> Result<Box<dyn LinearMemory>, String> {
		let accessible_bytes = (u64::from(ty.limits().min())) << WASM_PAGE_SHIFT;
		let mapped_bytes = if let Some(reserved) = reserved_size_in_bytes {
			reserved
		} else {
			accessible_bytes
		}
			.checked_add(guard_size_in_bytes)
			.ok_or_else(|| "Guard size overflowed u64".to_string())?;

		assert!(accessible_bytes <= mapped_bytes);

		let mut address: mach_vm_address_t = 0;
		let result = unsafe {
			mach_vm_allocate(
				self.task,
				&mut address,
				mapped_bytes,
				1 | 2,
			)
		};
		if result != KERN_SUCCESS {
			return Err(format!("mach_vm_allocate_returned: {}", result));
		}

		// Block out the guard pages
		change_protection(
			address + accessible_bytes,
			mapped_bytes - accessible_bytes,
			VM_PROT_NONE,
		);

		let result = Box::new(MachMemory {
			address,
			mapped_bytes,
			guard_bytes: guard_size_in_bytes,
			wasm_pages: Mutex::new(ty.limits().min()),
			wasm_pages_max: ty.limits().max(),
		});
		Ok(result)
	}
}

unsafe impl LinearMemory for MachMemory {
    fn size(&self) -> u32 {
		*self.wasm_pages.lock().unwrap()
	}

    fn maximum(&self) -> Option<u32> {
		self.wasm_pages_max
	}

    fn grow(&self, delta: u32) -> Option<u32> {
		let mut wasm_pages = self.wasm_pages.lock().unwrap();
		let new_page_num = wasm_pages.checked_add(delta)?;
		match self.wasm_pages_max {
			Some(max) if new_page_num > max => return None,
			_ => (),
		}
		let new_bytes = (new_page_num as u64) << WASM_PAGE_SHIFT;
		// for now we do not support reallocating
		assert!(new_bytes.checked_add(self.guard_bytes)? > self.mapped_bytes);
		*wasm_pages = new_page_num;
		change_protection(self.address, new_bytes, VM_PROT_DEFAULT);
		Some(new_page_num)
	}

    fn as_ptr(&self) -> *mut u8 {
		self.address as _
	}
}

fn change_protection(addr: mach_vm_address_t, size: mach_vm_size_t, prot: vm_prot_t) {
	let result = unsafe {
		mach_vm_protect(
			mach_task_self(),
			addr,
			size,
			0,
			prot,
		)
	};
	assert_eq!(result, 0);
}
