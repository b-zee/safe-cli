// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::AppExchangeInfo;
// use crate::ffi::ipc::req as ffi;

// use ffi_utils::{vec_into_raw_parts, ReprC, StringError};
use serde::{Deserialize, Serialize};



/// Represents an authorisation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthReq {
    /// The application identifier for this request
    pub app: AppExchangeInfo,
    // /// `true` if the app wants dedicated container for itself. `false` otherwise.
    // pub app_container: bool,
    // /// Stores app permissions, e.g. allowing to work with the user's coin balance.
    // pub app_permissions: AppPermissions,
    // /// The list of containers the app wishes to access (and desired permissions).
    // pub containers: HashMap<String, ContainerPermissions>,
}

// impl AuthReq {
//     /// Construct FFI wrapper for the native Rust object, consuming self.
//     pub fn into_repr_c(self) -> Result<ffi::AuthReq, IpcError> {
//         let Self {
//             app,
//             app_container,
//             app_permissions,
//             containers,
//         } = self;

//         let containers = containers_into_vec(containers).map_err(StringError::from)?;
//         let (containers_ptr, containers_len) = vec_into_raw_parts(containers);

//         Ok(ffi::AuthReq {
//             app: app.into_repr_c()?,
//             app_container,
//             app_permission_transfer_money: app_permissions.transfer_money,
//             app_permission_perform_mutations: app_permissions.perform_mutations,
//             app_permission_read_balance: app_permissions.read_balance,
//             containers: containers_ptr,
//             containers_len,
//         })
//     }
// }

// impl ReprC for AuthReq {
//     type C = *const ffi::AuthReq;
//     type Error = IpcError;

//     /// Constructs the object from the FFI counterpart.
//     ///
//     /// After calling this function, the subobjects memory is owned by the resulting object.
//     unsafe fn clone_from_repr_c(repr_c: Self::C) -> Result<Self, Self::Error> {
//         Ok(Self {
//             app: AppExchangeInfo::clone_from_repr_c(&(*repr_c).app)?,
//             app_container: (*repr_c).app_container,
//             app_permissions: AppPermissions {
//                 transfer_money: (*repr_c).app_permission_transfer_money,
//                 perform_mutations: (*repr_c).app_permission_perform_mutations,
//                 read_balance: (*repr_c).app_permission_read_balance,
//             },
//             containers: containers_from_repr_c((*repr_c).containers, (*repr_c).containers_len)?,
//         })
//     }
// }
