//! `pingo-listener` — gateway infrastructure layer.
//!
//! Assembles the Pingora server and its services: the public `HttpProxy`
//! service that runs the request pipeline, and the admin `ServeHttp` service.
//! Pingora is wired in starting at task T013 (this scaffold compiles without it).
