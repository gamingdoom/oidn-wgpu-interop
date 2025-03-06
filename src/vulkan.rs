use ash::{khr, vk};
use oidn::sys::OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32;
use std::ptr;
use wgpu::hal::api::Vulkan;
use wgpu::hal::{CommandEncoder, vulkan};
use wgpu::util::align_to;
use wgpu::{BufferDescriptor, BufferUsages, DeviceDescriptor};
use windows::Win32::Foundation::GENERIC_ALL;

pub(crate) struct VulkanAllocation {
    memory: vk::DeviceMemory,
    wgpu_device: wgpu::Device,
}

impl Drop for VulkanAllocation {
    fn drop(&mut self) {
        unsafe {
            self.wgpu_device.as_hal::<Vulkan, _, _>(|device| {
                let device = device.unwrap();
                device.raw_device().free_memory(self.memory, None);
            })
        }
    }
}

impl crate::Device {
    pub(crate) async fn new_vulkan(
        adapter: &wgpu::Adapter,
        desc: &DeviceDescriptor<'_>,
        trace_path: Option<&std::path::Path>,
    ) -> Result<(Self, wgpu::Queue), crate::DeviceCreateError> {
        // # SAFETY: the raw handle is not manually destroyed.
        let adapter_vulkan_desc = unsafe {
            adapter.as_hal::<Vulkan, _, _>(|adapter| {
                adapter
                    .and_then(|adapter| {
                        (adapter
                            .physical_device_capabilities()
                            .supports_extension(khr::external_memory_win32::NAME)
                            // `get_physical_device_properties2` requires version >= 1.1
                        && adapter.shared_instance().raw_instance().get_physical_device_properties(adapter.raw_physical_device()).api_version >= vk::API_VERSION_1_1)
                        .then_some(adapter)
                    })
                    .map(|adapter| {
                        let mut id_properties = vk::PhysicalDeviceIDProperties::default();
                        adapter
                            .shared_instance()
                            .raw_instance()
                            .get_physical_device_properties2(
                                adapter.raw_physical_device(),
                                &mut vk::PhysicalDeviceProperties2::default()
                                    .push_next(&mut id_properties),
                            );
                        id_properties
                    })
            })
        };
        let Some(vk_desc) = adapter_vulkan_desc else {
            return Err(crate::DeviceCreateError::MissingFeature);
        };
        let device = unsafe {
            oidn::sys::oidnNewDeviceByUUID((&vk_desc.device_uuid) as *const _ as *const _)
        };
        if device.is_null() {
            return Err(crate::DeviceCreateError::OidnUnsupported);
        }
        let supported_memory_types = unsafe {
            oidn::sys::oidnCommitDevice(device);
            oidn::sys::oidnGetDeviceInt(device, b"externalMemoryTypes\0" as *const _ as _)
        } as i32;
        if supported_memory_types
            & OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32 as i32
            == 0
        {
            unsafe {
                oidn::sys::oidnReleaseDevice(device);
            }
            return Err(crate::DeviceCreateError::OidnImportUnsupported);
        }
        let oidn_device = unsafe {
            oidn::Device::from_raw(device)
        };
        let (wgpu_device, queue) = adapter
            .request_device(desc, trace_path)
            .await
            .map_err(|err| crate::DeviceCreateError::RequestDeviceError(err))?;
        Ok((
            crate::Device {
                wgpu_device,
                oidn_device,
                queue: queue.clone(),
                backend: crate::Backend::Vulkan,
            },
            queue,
        ))
    }
    pub(crate) fn allocate_shared_buffers_vulkan(
        &self,
        size: wgpu::BufferAddress,
    ) -> Result<crate::SharedBuffer, Option<()>> {
        assert_eq!(self.backend, crate::Backend::Vulkan);

        if size == 0 {
            return Err(None);
        }

        // # SAFETY: the raw handle is not manually destroyed.
        unsafe {
            self.wgpu_device.as_hal::<Vulkan, _, _>(|device| {
                let device = device.unwrap();
                let win_32_funcs = khr::external_memory_win32::Device::new(
                    device.shared_instance().raw_instance(),
                    device.raw_device(),
                );
                let vk_info = vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::CONCURRENT);

                let raw_buffer = device
                    .raw_device()
                    .create_buffer(&vk_info, None)
                    .map_err(|_| None)?;

                let req = device
                    .raw_device()
                    .get_buffer_memory_requirements(raw_buffer);

                let aligned_size = align_to(size, req.alignment);

                let mem_properties = device
                    .shared_instance()
                    .raw_instance()
                    .get_physical_device_memory_properties(device.raw_physical_device());

                let mut idx = None;

                let flags = vk::MemoryPropertyFlags::DEVICE_LOCAL;

                for (i, mem_ty) in mem_properties.memory_types_as_slice().iter().enumerate() {
                    let types_bits = 1 << i;
                    let is_required_memory_type = req.memory_type_bits & types_bits != 0;
                    let has_required_properties = mem_ty.property_flags & flags == flags;
                    if is_required_memory_type && has_required_properties {
                        idx = Some(i);
                        break;
                    }
                }

                let Some(idx) = idx else {
                    return Err(None);
                };

                let mut info = vk::MemoryAllocateInfo::default()
                    .allocation_size(aligned_size)
                    .memory_type_index(idx as u32);

                let mut export_alloc_info = vk::ExportMemoryAllocateInfo::default()
                    .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32_KHR);

                let mut win32_info =
                    vk::ExportMemoryWin32HandleInfoKHR::default().dw_access(GENERIC_ALL.0);

                info = info
                    .push_next(&mut win32_info)
                    .push_next(&mut export_alloc_info);

                let memory = match device.raw_device().allocate_memory(&info, None) {
                    Ok(memory) => memory,
                    Err(_) => return Err(None),
                };

                device
                    .raw_device()
                    .bind_buffer_memory(raw_buffer, memory, 0)
                    .map_err(|_| None)?;

                let handle = win_32_funcs
                    .get_memory_win32_handle(
                        &vk::MemoryGetWin32HandleInfoKHR::default()
                            .memory(memory)
                            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32_KHR),
                    )
                    .map_err(|_| None)?;

                let oidn_buffer = oidn::sys::oidnNewSharedBufferFromWin32Handle(
                    self.oidn_device.raw(),
                    OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32,
                    handle as *mut _,
                    ptr::null(),
                    size as usize,
                );
                if oidn_buffer.is_null() {
                    eprintln!("Failed to create oidn buffer",);
                    eprintln!("error: {:?}", self.oidn_device.get_error());
                    return Err(None);
                }
                let buf = vulkan::Device::buffer_from_raw(raw_buffer);
                let mut encoder = self.wgpu_device.create_command_encoder(&Default::default());
                // # SAFETY: the raw handle is not manually destroyed.
                encoder.as_hal_mut::<Vulkan, _, _>(|encoder| {
                    encoder.unwrap().clear_buffer(&buf, 0..size);
                });
                self.queue.submit([encoder.finish()]);
                // # SAFETY: Just initialized buffer, created it from the same device and made with
                // the manually mapped usages.
                let wgpu_buffer = self.wgpu_device.create_buffer_from_hal::<Vulkan>(
                    buf,
                    &BufferDescriptor {
                        label: None,
                        size,
                        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    },
                );
                Ok(crate::SharedBuffer {
                    _allocation: crate::Allocation::Vulkan {
                        _vulkan: VulkanAllocation {
                            memory,
                            wgpu_device: self.wgpu_device.clone(),
                        },
                    },
                    wgpu_buffer,
                    oidn_buffer: self.oidn_device.create_buffer_from_raw(oidn_buffer),
                })
            })
        }
    }
}
