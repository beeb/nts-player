//! `ESPHome` native API messages used by the device.
//!
//! Field numbers and message IDs come from the `ESPHome` 2026.10.2 `api.proto` (MIT):
//! <https://github.com/esphome/esphome/blob/2026.10.2/esphome/components/api/api.proto>.
//! Only the fields the device uses are declared. Prost skips the others when decoding.

use prost::Message;

pub const API_VERSION_MAJOR: u32 = 1;
pub const API_VERSION_MINOR: u32 = 19;

/// `ESPHome` version reported to HA.
pub const ESPHOME_VERSION: &str = "2026.10.2";

pub const HELLO_REQUEST: u32 = 1;
pub const AUTHENTICATION_REQUEST: u32 = 3;
pub const DISCONNECT_REQUEST: u32 = 5;
pub const PING_REQUEST: u32 = 7;
pub const DEVICE_INFO_REQUEST: u32 = 9;
pub const LIST_ENTITIES_REQUEST: u32 = 11;
pub const SUBSCRIBE_STATES_REQUEST: u32 = 20;
pub const SWITCH_COMMAND_REQUEST: u32 = 33;
pub const DEVICE_CAPABILITIES_REQUEST: u32 = 149;
pub const NUMBER_COMMAND_REQUEST: u32 = 51;
pub const SELECT_COMMAND_REQUEST: u32 = 54;
pub const BUTTON_COMMAND_REQUEST: u32 = 62;

/// `NumberMode::NUMBER_MODE_SLIDER`.
pub const NUMBER_MODE_SLIDER: i32 = 2;

pub trait Typed: Message {
    const ID: u32;
}

#[derive(Clone, PartialEq, Message)]
pub struct HelloResponse {
    #[prost(uint32, tag = "1")]
    pub api_version_major: u32,
    #[prost(uint32, tag = "2")]
    pub api_version_minor: u32,
    #[prost(string, tag = "3")]
    pub server_info: String,
    #[prost(string, tag = "4")]
    pub name: String,
}

impl Typed for HelloResponse {
    const ID: u32 = 2;
}

#[derive(Clone, PartialEq, Message)]
pub struct AuthenticationResponse {
    #[prost(bool, tag = "1")]
    pub invalid_password: bool,
}

impl Typed for AuthenticationResponse {
    const ID: u32 = 4;
}

#[derive(Clone, PartialEq, Message)]
pub struct DisconnectResponse {}

impl Typed for DisconnectResponse {
    const ID: u32 = 6;
}

#[derive(Clone, PartialEq, Message)]
pub struct PingResponse {}

impl Typed for PingResponse {
    const ID: u32 = 8;
}

#[derive(Clone, PartialEq, Message)]
pub struct DeviceInfoResponse {
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub mac_address: String,
    #[prost(string, tag = "4")]
    pub esphome_version: String,
    #[prost(string, tag = "6")]
    pub model: String,
    #[prost(string, tag = "12")]
    pub manufacturer: String,
    #[prost(string, tag = "13")]
    pub friendly_name: String,
}

impl Typed for DeviceInfoResponse {
    const ID: u32 = 10;
}

#[derive(Clone, PartialEq, Message)]
pub struct DeviceCapabilitiesResponse {}

impl Typed for DeviceCapabilitiesResponse {
    const ID: u32 = 150;
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesSwitchResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

impl Typed for ListEntitiesSwitchResponse {
    const ID: u32 = 17;
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesSelectResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(string, repeated, tag = "6")]
    pub options: Vec<String>,
}

impl Typed for ListEntitiesSelectResponse {
    const ID: u32 = 52;
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesNumberResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(float, tag = "6")]
    pub min_value: f32,
    #[prost(float, tag = "7")]
    pub max_value: f32,
    #[prost(float, tag = "8")]
    pub step: f32,
    #[prost(int32, tag = "12")]
    pub mode: i32,
}

impl Typed for ListEntitiesNumberResponse {
    const ID: u32 = 49;
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesButtonResponse {
    #[prost(string, tag = "1")]
    pub object_id: String,
    #[prost(fixed32, tag = "2")]
    pub key: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(string, tag = "5")]
    pub icon: String,
}

impl Typed for ListEntitiesButtonResponse {
    const ID: u32 = 61;
}

#[derive(Clone, PartialEq, Message)]
pub struct ListEntitiesDoneResponse {}

impl Typed for ListEntitiesDoneResponse {
    const ID: u32 = 19;
}

#[derive(Clone, PartialEq, Message)]
pub struct SwitchStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
}

impl Typed for SwitchStateResponse {
    const ID: u32 = 26;
}

#[derive(Clone, PartialEq, Message)]
pub struct SelectStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(string, tag = "2")]
    pub state: String,
}

impl Typed for SelectStateResponse {
    const ID: u32 = 53;
}

#[derive(Clone, PartialEq, Message)]
pub struct NumberStateResponse {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(float, tag = "2")]
    pub state: f32,
}

impl Typed for NumberStateResponse {
    const ID: u32 = 50;
}

#[derive(Clone, PartialEq, Message)]
pub struct SwitchCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(bool, tag = "2")]
    pub state: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct SelectCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(string, tag = "2")]
    pub state: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct NumberCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
    #[prost(float, tag = "2")]
    pub state: f32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ButtonCommandRequest {
    #[prost(fixed32, tag = "1")]
    pub key: u32,
}
