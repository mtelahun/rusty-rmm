#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EndpointRegistration {
    #[prost(string, tag = "1")]
    pub hostname: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub system_uuid: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EndpointUpdate {
    #[prost(message, optional, tag = "1")]
    pub id: ::core::option::Option<RustyRmmId>,
    #[prost(string, tag = "2")]
    pub hostname: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub system_uuid: ::prost::alloc::string::String,
    #[prost(message, optional, tag = "4")]
    pub os: ::core::option::Option<OsInfo>,
    #[prost(message, optional, tag = "5")]
    pub cpu: ::core::option::Option<CpuInfo>,
    #[prost(message, optional, tag = "6")]
    pub memory: ::core::option::Option<MemInfo>,
    #[prost(message, optional, tag = "7")]
    pub disks: ::core::option::Option<DiskInfo>,
    #[prost(message, optional, tag = "8")]
    pub net: ::core::option::Option<NetInfo>,
    #[prost(message, optional, tag = "9")]
    pub updates: ::core::option::Option<UpdateStatus>,
    #[prost(message, optional, tag = "10")]
    pub client_version: ::core::option::Option<ClientVer>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RustyRmmId {
    #[prost(string, tag = "1")]
    pub uuid: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct OsInfo {
    #[prost(string, tag = "1")]
    pub full_name: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub family: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub version: ::prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub virt_system: ::prost::alloc::string::String,
    #[prost(string, tag = "5")]
    pub virt_role: ::prost::alloc::string::String,
    #[prost(string, tag = "6")]
    pub tz: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CpuInfo {
    #[prost(uint32, tag = "1")]
    pub core_count: u32,
    #[prost(uint32, tag = "2")]
    pub thread_count: u32,
    #[prost(message, repeated, tag = "3")]
    pub cpus: ::prost::alloc::vec::Vec<Cpu>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Cpu {
    #[prost(string, tag = "1")]
    pub name: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub vendor_id: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub brand: ::prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub frequency: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MemInfo {
    #[prost(uint64, tag = "1")]
    pub total: u64,
    #[prost(uint64, tag = "2")]
    pub used: u64,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DiskInfo {
    #[prost(message, repeated, tag = "1")]
    pub disks: ::prost::alloc::vec::Vec<Disk>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Disk {
    #[prost(string, tag = "1")]
    pub name: ::prost::alloc::string::String,
    #[prost(enumeration = "DiskType", tag = "2")]
    pub disk_type: i32,
    #[prost(string, tag = "3")]
    pub filesystem: ::prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub mount_point: ::prost::alloc::string::String,
    #[prost(uint64, tag = "5")]
    pub size: u64,
    #[prost(uint64, tag = "6")]
    pub free: u64,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct NetInfo {
    #[prost(message, repeated, tag = "1")]
    pub ifs: ::prost::alloc::vec::Vec<NetInterface>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct NetInterface {
    #[prost(string, tag = "1")]
    pub name: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub mac: ::prost::alloc::string::String,
    #[prost(message, repeated, tag = "4")]
    pub ip4: ::prost::alloc::vec::Vec<Ip4Addr>,
    #[prost(message, repeated, tag = "5")]
    pub ip6: ::prost::alloc::vec::Vec<Ip6Addr>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Ip4Addr {
    #[prost(string, tag = "1")]
    pub ip: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Ip6Addr {
    #[prost(string, tag = "1")]
    pub ip: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UpdateStatus {
    #[prost(uint32, tag = "1")]
    pub security: u32,
    #[prost(uint32, tag = "2")]
    pub regular: u32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ClientVer {
    #[prost(string, tag = "1")]
    pub version: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EndpointRegistrationResponse {
    #[prost(enumeration = "ResponseStatus", tag = "1")]
    pub status: i32,
    #[prost(message, optional, tag = "2")]
    pub id: ::core::option::Option<RustyRmmId>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EndpointUpdateResponse {
    #[prost(enumeration = "ResponseStatus", tag = "1")]
    pub status: i32,
    #[prost(message, optional, tag = "2")]
    pub id: ::core::option::Option<RustyRmmId>,
    #[prost(uint32, tag = "3")]
    pub updated: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum DiskType {
    TypeUnknown = 0,
    TypeHdd = 1,
    TypeSsd = 2,
}
impl DiskType {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            DiskType::TypeUnknown => "TYPE_UNKNOWN",
            DiskType::TypeHdd => "TYPE_HDD",
            DiskType::TypeSsd => "TYPE_SSD",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "TYPE_UNKNOWN" => Some(Self::TypeUnknown),
            "TYPE_HDD" => Some(Self::TypeHdd),
            "TYPE_SSD" => Some(Self::TypeSsd),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ResponseStatus {
    StatusUnspecified = 0,
    StatusOk = 1,
    StatusErr = 2,
}
impl ResponseStatus {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            ResponseStatus::StatusUnspecified => "STATUS_UNSPECIFIED",
            ResponseStatus::StatusOk => "STATUS_OK",
            ResponseStatus::StatusErr => "STATUS_ERR",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "STATUS_UNSPECIFIED" => Some(Self::StatusUnspecified),
            "STATUS_OK" => Some(Self::StatusOk),
            "STATUS_ERR" => Some(Self::StatusErr),
            _ => None,
        }
    }
}
/// Generated client implementations.
pub mod registration_service_client {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::http::Uri;
    use tonic::codegen::*;
    #[derive(Debug, Clone)]
    pub struct RegistrationServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl RegistrationServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> RegistrationServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> RegistrationServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<http::Request<tonic::body::BoxBody>>>::Error:
                Into<StdError> + Send + Sync,
        {
            RegistrationServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        pub async fn register_endpoint(
            &mut self,
            request: impl tonic::IntoRequest<super::EndpointRegistration>,
        ) -> std::result::Result<tonic::Response<super::EndpointRegistrationResponse>, tonic::Status>
        {
            self.inner.ready().await.map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Unknown,
                    format!("Service was not ready: {}", e.into()),
                )
            })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/endpoint_registration.RegistrationService/RegisterEndpoint",
            );
            let mut req = request.into_request();
            req.extensions_mut().insert(GrpcMethod::new(
                "endpoint_registration.RegistrationService",
                "RegisterEndpoint",
            ));
            self.inner.unary(req, path, codec).await
        }
        pub async fn update_endpoint(
            &mut self,
            request: impl tonic::IntoRequest<super::EndpointUpdate>,
        ) -> std::result::Result<tonic::Response<super::EndpointUpdateResponse>, tonic::Status>
        {
            self.inner.ready().await.map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Unknown,
                    format!("Service was not ready: {}", e.into()),
                )
            })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/endpoint_registration.RegistrationService/UpdateEndpoint",
            );
            let mut req = request.into_request();
            req.extensions_mut().insert(GrpcMethod::new(
                "endpoint_registration.RegistrationService",
                "UpdateEndpoint",
            ));
            self.inner.unary(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod registration_service_server {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with RegistrationServiceServer.
    #[async_trait]
    pub trait RegistrationService: Send + Sync + 'static {
        async fn register_endpoint(
            &self,
            request: tonic::Request<super::EndpointRegistration>,
        ) -> std::result::Result<tonic::Response<super::EndpointRegistrationResponse>, tonic::Status>;
        async fn update_endpoint(
            &self,
            request: tonic::Request<super::EndpointUpdate>,
        ) -> std::result::Result<tonic::Response<super::EndpointUpdateResponse>, tonic::Status>;
    }
    #[derive(Debug)]
    pub struct RegistrationServiceServer<T: RegistrationService> {
        inner: _Inner<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    struct _Inner<T>(Arc<T>);
    impl<T: RegistrationService> RegistrationServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            let inner = _Inner(inner);
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(inner: T, interceptor: F) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for RegistrationServiceServer<T>
    where
        T: RegistrationService,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = self.inner.clone();
            match req.uri().path() {
                "/endpoint_registration.RegistrationService/RegisterEndpoint" => {
                    #[allow(non_camel_case_types)]
                    struct RegisterEndpointSvc<T: RegistrationService>(pub Arc<T>);
                    impl<T: RegistrationService>
                        tonic::server::UnaryService<super::EndpointRegistration>
                        for RegisterEndpointSvc<T>
                    {
                        type Response = super::EndpointRegistrationResponse;
                        type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::EndpointRegistration>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as RegistrationService>::register_endpoint(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = RegisterEndpointSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/endpoint_registration.RegistrationService/UpdateEndpoint" => {
                    #[allow(non_camel_case_types)]
                    struct UpdateEndpointSvc<T: RegistrationService>(pub Arc<T>);
                    impl<T: RegistrationService> tonic::server::UnaryService<super::EndpointUpdate>
                        for UpdateEndpointSvc<T>
                    {
                        type Response = super::EndpointUpdateResponse;
                        type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::EndpointUpdate>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as RegistrationService>::update_endpoint(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = UpdateEndpointSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => Box::pin(async move {
                    Ok(http::Response::builder()
                        .status(200)
                        .header("grpc-status", "12")
                        .header("content-type", "application/grpc")
                        .body(empty_body())
                        .unwrap())
                }),
            }
        }
    }
    impl<T: RegistrationService> Clone for RegistrationServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    impl<T: RegistrationService> Clone for _Inner<T> {
        fn clone(&self) -> Self {
            Self(Arc::clone(&self.0))
        }
    }
    impl<T: std::fmt::Debug> std::fmt::Debug for _Inner<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
    impl<T: RegistrationService> tonic::server::NamedService for RegistrationServiceServer<T> {
        const NAME: &'static str = "endpoint_registration.RegistrationService";
    }
}
