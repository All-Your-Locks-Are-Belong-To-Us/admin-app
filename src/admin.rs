use core::{convert::TryInto, marker::PhantomData};
use ctaphid_dispatch::app::{self as hid, Command as HidCommand, Message};
use ctaphid_dispatch::command::VendorCommand;
use apdu_dispatch::{Command, command, response, app as apdu};
use apdu_dispatch::iso7816::Status;
use trussed::{
    syscall,
    Client as TrussedClient,
};

const UPDATE: VendorCommand = VendorCommand::H51;
const REBOOT: VendorCommand = VendorCommand::H53;
const RNG: VendorCommand = VendorCommand::H60;
const VERSION: VendorCommand = VendorCommand::H61;
const UUID: VendorCommand = VendorCommand::H62;

pub trait Reboot {
    /// Reboots the device.
    fn reboot() -> !;

    /// Reboots the device.
    ///
    /// Presuming the device has a separate mode of operation that
    /// allows updating its firmware (for instance, a bootloader),
    /// reboots the device into this mode.
    fn reboot_to_firmware_update() -> !;

    /// Reboots the device.
    ///
    /// Presuming the device has a separate destructive but more
    /// reliable way of rebooting into the firmware mode of operation,
    /// does so.
    fn reboot_to_firmware_update_destructive() -> !;
}

pub struct App<T, R>
where T: TrussedClient,
      R: Reboot,
{
    trussed: T,
    uuid: [u8; 16],
    version: u32,
    boot_interface: PhantomData<R>,
}

impl<T, R> App<T, R>
where T: TrussedClient,
      R: Reboot,
{
    pub fn new(client: T, uuid: [u8; 16], version: u32) -> Self {
        Self { trussed: client, uuid, version, boot_interface: PhantomData }
    }

    fn user_present(&mut self) -> bool {
        let user_present = syscall!(self.trussed.confirm_user_present(15_000)).result;
        user_present.is_ok()
    }


}

impl<T, R> hid::App for App<T, R>
where T: TrussedClient,
      R: Reboot
{
    fn commands(&self) -> &'static [HidCommand] {
        &[
            HidCommand::Wink,
            HidCommand::Vendor(UPDATE),
            HidCommand::Vendor(REBOOT),
            HidCommand::Vendor(RNG),
            HidCommand::Vendor(VERSION),
            HidCommand::Vendor(UUID),
        ]
    }

    fn call(&mut self, command: HidCommand, input_data: &Message, response: &mut Message) -> hid::AppResult {
        match command {
            HidCommand::Vendor(REBOOT) => R::reboot(),
            HidCommand::Vendor(RNG) => {
                // Fill the HID packet (57 bytes)
                response.extend_from_slice(
                    &syscall!(self.trussed.random_bytes(57)).bytes.as_slice()
                ).ok();
            }
            HidCommand::Vendor(UPDATE) => {
                if self.user_present() {
                    if input_data.len() > 0 && input_data[0] == 0x01 {
                        R::reboot_to_firmware_update_destructive();
                    } else {
                        R::reboot_to_firmware_update();
                    }
                } else {
                    return Err(hid::Error::InvalidLength);
                }
            }
            HidCommand::Vendor(UUID) => {
                // Get UUID
                response.extend_from_slice(&self.uuid).ok();
            }
            HidCommand::Vendor(VERSION) => {
                // GET VERSION
                response.extend_from_slice(&self.version.to_be_bytes()).ok();
            }
            HidCommand::Wink => {
                syscall!(self.trussed.wink(core::time::Duration::from_secs(10)));
            }
            _ => {
                return Err(hid::Error::InvalidCommand);
            }
        }
        Ok(())
    }
}

impl<T, R> iso7816::App for App<T, R>
where T: TrussedClient,
      R: Reboot
{
    // Solo management app
    fn aid(&self) -> iso7816::Aid {
        iso7816::Aid::new(&[ 0xA0, 0x00, 0x00, 0x08, 0x47, 0x00, 0x00, 0x00, 0x01])
    }
}

impl<T, R> apdu::App<{command::SIZE}, {response::SIZE}> for App<T, R>
where T: TrussedClient,
      R: Reboot
{

    fn select(&mut self, _apdu: &Command, _reply: &mut response::Data) -> apdu::Result {
        Ok(())
    }

    fn deselect(&mut self) {}

    fn call(&mut self, interface: apdu::Interface, apdu: &Command, reply: &mut response::Data) -> apdu::Result {
        let instruction: u8 = apdu.instruction().into();

        let command: VendorCommand = instruction.try_into().map_err(|_e| Status::InstructionNotSupportedOrInvalid)?;

        match command {
            REBOOT => R::reboot(),
            RNG => {
                // Random bytes
                reply.extend_from_slice(&syscall!(self.trussed.random_bytes(57)).bytes.as_slice()).ok();
            }
            UPDATE => {
                // Boot to mcuboot (only when contact interface)
                if interface == apdu::Interface::Contact && self.user_present()
                {
                    if apdu.p1 == 0x01 {
                        R::reboot_to_firmware_update_destructive();
                    } else {
                        R::reboot_to_firmware_update();
                    }
                }
                return Err(Status::ConditionsOfUseNotSatisfied);
            }
            UUID => {
                // Get UUID
                reply.extend_from_slice(&self.uuid).ok();
            }
            VERSION => {
                // Get version
                reply.extend_from_slice(&self.version.to_be_bytes()[..]).ok();
            }

            _ => return Err(Status::InstructionNotSupportedOrInvalid),

        }
        Ok(())

    }
}

