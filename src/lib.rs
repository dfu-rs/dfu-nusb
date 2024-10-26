use std::time::Duration;

use dfu_core::{
    asynchronous::{DfuASync, DfuAsyncIo},
    functional_descriptor::FunctionalDescriptor,
    sync::DfuSync,
    DfuIo, DfuProtocol,
};
use nusb::transfer::{Control, ControlIn, ControlOut, ControlType, Recipient, TransferError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Device not found")]
    DeviceNotFound,
    #[error("Functional Desciptor not found")]
    FunctionalDescriptorNotFound,
    #[error("Alternative setting not found")]
    AltSettingNotFound,
    #[error(transparent)]
    FunctionalDescriptor(#[from] dfu_core::functional_descriptor::Error),
    #[error(transparent)]
    Dfu(#[from] dfu_core::Error),
    #[error(transparent)]
    Nusb(#[from] nusb::Error),
    #[error(transparent)]
    Transfer(#[from] TransferError),
}

pub struct DfuNusb {
    device: nusb::Device,
    interface: nusb::Interface,
    descriptor: FunctionalDescriptor,
    protocol: dfu_core::DfuProtocol<dfu_core::memory_layout::MemoryLayout>,
}

impl DfuNusb {
    /// Open a device
    pub fn open(device: nusb::Device, interface: nusb::Interface, alt: u8) -> Result<Self, Error> {
        interface.set_alt_setting(alt)?;
        let descriptor = interface
            .descriptors()
            .find_map(|alt| {
                alt.descriptors()
                    .find_map(|d| FunctionalDescriptor::from_bytes(&d))
            })
            .ok_or(Error::FunctionalDescriptorNotFound)??;
        let alt = interface
            .descriptors()
            .find(|a| a.alternate_setting() == alt)
            .ok_or(Error::AltSettingNotFound)?;

        let s = if let Some(index) = alt.string_index() {
            let lang = device
                .get_string_descriptor_supported_languages(Duration::from_secs(3))?
                .next()
                .unwrap_or_default();
            device
                .get_string_descriptor(index, lang, Duration::from_secs(3))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let protocol = DfuProtocol::new(&s, descriptor.dfu_version)?;

        Ok(Self {
            device,
            interface,
            descriptor,
            protocol,
        })
    }

    /// Wrap device in an *async* dfu helper
    pub fn into_async_dfu(self) -> DfuASync<Self, Error> {
        DfuASync::new(self)
    }

    /// Wrap device in an *sync* dfu helper
    pub fn into_sync_dfu(self) -> DfuSync<Self, Error> {
        DfuSync::new(self)
    }
}

fn split_request_type(request_type: u8) -> (ControlType, Recipient) {
    (
        match request_type >> 5 & 0x03 {
            0 => ControlType::Standard,
            1 => ControlType::Class,
            2 => ControlType::Vendor,
            _ => ControlType::Standard,
        },
        match request_type & 0x1f {
            0 => Recipient::Device,
            1 => Recipient::Interface,
            2 => Recipient::Endpoint,
            3 => Recipient::Other,
            _ => Recipient::Device,
        },
    )
}

impl DfuIo for DfuNusb {
    type Read = usize;
    type Write = usize;
    type Reset = ();
    type Error = Error;
    type MemoryLayout = dfu_core::memory_layout::MemoryLayout;

    fn read_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        buffer: &mut [u8],
    ) -> Result<Self::Read, Self::Error> {
        let (control_type, recipient) = split_request_type(request_type);
        let req = Control {
            control_type,
            recipient,
            request,
            value,
            index: self.interface.interface_number() as u16,
        };
        let r = self
            .interface
            .control_in_blocking(req, buffer, Duration::from_secs(3))?;
        Ok(r)
    }

    fn write_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        buffer: &[u8],
    ) -> Result<Self::Write, Self::Error> {
        let (control_type, recipient) = split_request_type(request_type);
        let req = Control {
            control_type,
            recipient,
            request,
            value,
            index: self.interface.interface_number() as u16,
        };
        let r = self
            .interface
            .control_out_blocking(req, buffer, Duration::from_secs(3))?;
        Ok(r)
    }

    fn usb_reset(&self) -> Result<Self::Reset, Self::Error> {
        self.device.reset()?;
        Ok(())
    }

    fn protocol(&self) -> &dfu_core::DfuProtocol<Self::MemoryLayout> {
        &self.protocol
    }

    fn functional_descriptor(&self) -> &FunctionalDescriptor {
        &self.descriptor
    }
}

impl DfuAsyncIo for DfuNusb {
    type Read = usize;
    type Write = usize;
    type Reset = ();
    type Error = Error;
    type MemoryLayout = dfu_core::memory_layout::MemoryLayout;

    async fn read_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        buffer: &mut [u8],
    ) -> Result<Self::Read, Self::Error> {
        let (control_type, recipient) = split_request_type(request_type);
        let req = ControlIn {
            control_type,
            recipient,
            request,
            value,
            index: self.interface.interface_number() as u16,
            length: buffer.len() as u16,
        };
        let r = self.interface.control_in(req).await.into_result()?;
        let len = buffer.len().min(r.len());
        buffer[0..len].copy_from_slice(&r[0..len]);
        Ok(len)
    }

    async fn write_control(
        &self,
        request_type: u8,
        request: u8,
        value: u16,
        buffer: &[u8],
    ) -> Result<Self::Write, Self::Error> {
        let (control_type, recipient) = split_request_type(request_type);
        let req = ControlOut {
            control_type,
            recipient,
            request,
            value,
            index: self.interface.interface_number() as u16,
            data: buffer,
        };
        let r = self.interface.control_out(req).await.into_result()?;
        Ok(r.actual_length())
    }

    async fn usb_reset(&self) -> Result<Self::Reset, Self::Error> {
        self.device.reset()?;
        Ok(())
    }

    fn protocol(&self) -> &dfu_core::DfuProtocol<Self::MemoryLayout> {
        &self.protocol
    }

    fn functional_descriptor(&self) -> &FunctionalDescriptor {
        &self.descriptor
    }
}
