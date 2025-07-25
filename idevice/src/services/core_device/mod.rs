// Jackson Coxson
// Ported from pymobiledevice3

use log::warn;

use crate::{
    xpc::{self, XPCObject},
    IdeviceError, ReadWrite, RemoteXpcClient,
};

mod app_service;
pub use app_service::*;

const CORE_SERVICE_VERSION: &str = "443.18";

pub struct CoreDeviceServiceClient<R: ReadWrite> {
    inner: RemoteXpcClient<R>,
}

impl<'a, R: ReadWrite + 'a> CoreDeviceServiceClient<R> {
    pub async fn new(inner: R) -> Result<Self, IdeviceError> {
        let mut client = RemoteXpcClient::new(inner).await?;
        client.do_handshake().await?;
        Ok(Self { inner: client })
    }

    pub fn box_inner(self) -> CoreDeviceServiceClient<Box<dyn ReadWrite + 'a>> {
        CoreDeviceServiceClient {
            inner: self.inner.box_inner(),
        }
    }

    pub async fn invoke(
        &mut self,
        feature: impl Into<String>,
        input: Option<plist::Dictionary>,
    ) -> Result<plist::Value, IdeviceError> {
        let feature = feature.into();
        let input = input.unwrap_or_default();

        let mut req = xpc::Dictionary::new();
        req.insert(
            "CoreDevice.CoreDeviceDDIProtocolVersion".into(),
            XPCObject::Int64(0),
        );
        req.insert("CoreDevice.action".into(), xpc::Dictionary::new().into());
        req.insert(
            "CoreDevice.coreDeviceVersion".into(),
            create_xpc_version_from_string(CORE_SERVICE_VERSION).into(),
        );
        req.insert(
            "CoreDevice.deviceIdentifier".into(),
            XPCObject::String(uuid::Uuid::new_v4().to_string()),
        );
        req.insert(
            "CoreDevice.featureIdentifier".into(),
            XPCObject::String(feature),
        );
        req.insert(
            "CoreDevice.input".into(),
            plist::Value::Dictionary(input).into(),
        );
        req.insert(
            "CoreDevice.invocationIdentifier".into(),
            XPCObject::String(uuid::Uuid::new_v4().to_string()),
        );

        self.inner.send_object(req, true).await?;
        let res = self.inner.recv().await?;
        let mut res = match res {
            plist::Value::Dictionary(d) => d,
            _ => {
                warn!("XPC response was not a dictionary");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        let res = match res.remove("CoreDevice.output") {
            Some(r) => r,
            None => {
                warn!("XPC response did not have an output");
                return Err(IdeviceError::UnexpectedResponse);
            }
        };

        Ok(res)
    }
}

fn create_xpc_version_from_string(version: impl Into<String>) -> xpc::Dictionary {
    let version: String = version.into();
    let mut collected_version = Vec::new();
    version.split('.').for_each(|x| {
        if let Ok(x) = x.parse() {
            collected_version.push(XPCObject::UInt64(x));
        }
    });

    let mut res = xpc::Dictionary::new();
    res.insert(
        "originalComponentsCount".into(),
        XPCObject::Int64(collected_version.len() as i64),
    );
    res.insert("components".into(), XPCObject::Array(collected_version));
    res.insert("stringValue".into(), XPCObject::String(version));
    res
}
