// This file is generated. Do not edit
// @generated

// https://github.com/Manishearth/rust-clippy/issues/702
#![allow(unknown_lints)]
#![allow(clippy::all)]

#![cfg_attr(rustfmt, rustfmt_skip)]

#![allow(box_pointers)]
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(trivial_casts)]
#![allow(unsafe_code)]
#![allow(unused_imports)]
#![allow(unused_results)]


// interface

pub trait Termcast {
    fn list(&self, o: ::grpc::RequestOptions, p: super::termcast::ListRequest) -> ::grpc::SingleResponse<super::termcast::ListResponse>;

    fn watch(&self, o: ::grpc::RequestOptions, p: super::termcast::WatchRequest) -> ::grpc::StreamingResponse<super::termcast::WatchProgress>;
}

// client

pub struct TermcastClient {
    grpc_client: ::std::sync::Arc<::grpc::Client>,
    method_List: ::std::sync::Arc<::grpc::rt::MethodDescriptor<super::termcast::ListRequest, super::termcast::ListResponse>>,
    method_Watch: ::std::sync::Arc<::grpc::rt::MethodDescriptor<super::termcast::WatchRequest, super::termcast::WatchProgress>>,
}

impl ::grpc::ClientStub for TermcastClient {
    fn with_client(grpc_client: ::std::sync::Arc<::grpc::Client>) -> Self {
        TermcastClient {
            grpc_client: grpc_client,
            method_List: ::std::sync::Arc::new(::grpc::rt::MethodDescriptor {
                name: "/Termcast/List".to_string(),
                streaming: ::grpc::rt::GrpcStreaming::Unary,
                req_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                resp_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
            }),
            method_Watch: ::std::sync::Arc::new(::grpc::rt::MethodDescriptor {
                name: "/Termcast/Watch".to_string(),
                streaming: ::grpc::rt::GrpcStreaming::ServerStreaming,
                req_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                resp_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
            }),
        }
    }
}

impl Termcast for TermcastClient {
    fn list(&self, o: ::grpc::RequestOptions, p: super::termcast::ListRequest) -> ::grpc::SingleResponse<super::termcast::ListResponse> {
        self.grpc_client.call_unary(o, p, self.method_List.clone())
    }

    fn watch(&self, o: ::grpc::RequestOptions, p: super::termcast::WatchRequest) -> ::grpc::StreamingResponse<super::termcast::WatchProgress> {
        self.grpc_client.call_server_streaming(o, p, self.method_Watch.clone())
    }
}

// server

pub struct TermcastServer;


impl TermcastServer {
    pub fn new_service_def<H : Termcast + 'static + Sync + Send + 'static>(handler: H) -> ::grpc::rt::ServerServiceDefinition {
        let handler_arc = ::std::sync::Arc::new(handler);
        ::grpc::rt::ServerServiceDefinition::new("/Termcast",
            vec![
                ::grpc::rt::ServerMethod::new(
                    ::std::sync::Arc::new(::grpc::rt::MethodDescriptor {
                        name: "/Termcast/List".to_string(),
                        streaming: ::grpc::rt::GrpcStreaming::Unary,
                        req_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                        resp_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                    }),
                    {
                        let handler_copy = handler_arc.clone();
                        ::grpc::rt::MethodHandlerUnary::new(move |o, p| handler_copy.list(o, p))
                    },
                ),
                ::grpc::rt::ServerMethod::new(
                    ::std::sync::Arc::new(::grpc::rt::MethodDescriptor {
                        name: "/Termcast/Watch".to_string(),
                        streaming: ::grpc::rt::GrpcStreaming::ServerStreaming,
                        req_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                        resp_marshaller: Box::new(::grpc::protobuf::MarshallerProtobuf),
                    }),
                    {
                        let handler_copy = handler_arc.clone();
                        ::grpc::rt::MethodHandlerServerStreaming::new(move |o, p| handler_copy.watch(o, p))
                    },
                ),
            ],
        )
    }
}
