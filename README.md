# Oidn wgpu interoperability

---

A helper library to create shared buffers between oidn and
wgpu.

## Getting started

### Creating the device

Simply replace the `adapter.request_device` call with
`oidn_wgpu_interop::Device::new` (using
`adapter.request_device` only if the other fails). You are
then able to call `device.wgpu_device` to get the created
wgpu device and `device.oidn_device` to get the oidn device.

### Creating shared buffers

To create a shared buffer call
`device.allocate_shared_buffers`. The shared buffer may be
used with usages
`BufferUsages::COPY_SRC | BufferUsages::COPY_DST`. To get
the wgpu buffer call `buffer.wgpu_buffer` and to get the
oidn buffer call `buffer.oidn_buffer`. It is recommended to
minimise the number of shared buffers that exist at a given
time due to them each requiring a separate allocation.

## Platform Support

Currently, this only supports DirectX12 and Vulkan (on
Windows using `VK_KHR_external_memory_win32`). This can be
expanded to Vulkan (on Linux using
`VK_KHR_external_memory_fd`) and Metal. Due to some devices
being unsupported by oidn it is recommended to support a
mode that copies to the cpu and then into an oidn buffer
anyway.