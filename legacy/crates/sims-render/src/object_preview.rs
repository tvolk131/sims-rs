//! Shared object-preview helpers recovered from 4384c8 and 4e5544/4e572c.
//!
//! The fitting policy and drawing-target ownership are separate from 43878f's
//! resource traversal and sprite drawing. A missing renderer must leave the
//! fitting request pending; an empty rectangle is a real result, not a fallback.
pub mod fit;
pub mod software;
pub mod surface;
