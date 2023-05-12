/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use allocative::Allocative;
use dupe::Dupe;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::values::Freeze;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;

use crate::interpreter::rule_defs::artifact_tagging::TaggedCommandLine;
use crate::interpreter::rule_defs::artifact_tagging::TaggedValue;
use crate::interpreter::rule_defs::cmd_args::value_as::ValueAsCommandLineLike;

/// ArtifactTag allows wrapping input and output artifacts in a command line with tags. Those tags
/// will be made visible to artifact visitors. The tags themselves don't have meaning on their own,
/// but they can be compared to each other, which allows grouping inputs and outputs in meaningful
/// categories. This is notably used for dep files to associate inputs tracked by a dep file with
/// the dep file itself.
#[derive(
    Debug,
    Clone,
    Dupe,
    Freeze,
    Trace,
    ProvidesStaticType,
    NoSerialize,
    Allocative
)]
pub struct ArtifactTag {
    #[cfg_attr(feature = "gazebo_lint", allow(gazebo_lint_arc_on_dupe))]
    #[freeze(identity)]
    identity: Arc<()>,
}

impl ArtifactTag {
    pub fn new() -> Self {
        Self {
            identity: Arc::new(()),
        }
    }
}

impl fmt::Display for ArtifactTag {
    fn fmt(&self, w: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(w, "ArtifactTag({:x})", Arc::as_ptr(&self.identity) as usize)
    }
}

impl PartialEq for ArtifactTag {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl Eq for ArtifactTag {}

impl Hash for ArtifactTag {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.write_usize(Arc::as_ptr(&self.identity) as usize);
    }
}

starlark_simple_value!(ArtifactTag);

impl<'v> StarlarkValue<'v> for ArtifactTag {
    starlark_type!("artifact_tag");

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(input_tag_methods)
    }

    fn equals(&self, other: Value<'v>) -> anyhow::Result<bool> {
        Ok(match other.downcast_ref::<Self>() {
            Some(other) => self == other,
            None => false,
        })
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> anyhow::Result<()> {
        Hash::hash(self, hasher);
        Ok(())
    }
}

#[starlark_module]
fn input_tag_methods(_: &mut MethodsBuilder) {
    fn tag_artifacts<'v>(
        this: &ArtifactTag,
        inner: Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<Value<'v>> {
        let value = TaggedValue::new(inner, this.dupe());

        Ok(if inner.as_command_line().is_some() {
            heap.alloc(TaggedCommandLine::new(value))
        } else {
            heap.alloc(value)
        })
    }

    fn tag_inputs<'v>(
        this: &ArtifactTag,
        inner: Value<'v>,
        heap: &'v Heap,
    ) -> anyhow::Result<Value<'v>> {
        let value = TaggedValue::inputs_only(inner, this.dupe());

        Ok(if inner.as_command_line().is_some() {
            heap.alloc(TaggedCommandLine::new(value))
        } else {
            heap.alloc(value)
        })
    }
}
