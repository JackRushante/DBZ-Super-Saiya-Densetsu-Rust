//! The lookup the redirectors share: the pack slot of an id in the image the machine runs.

use ssd_mod::Family;
use ssd_oracle::Ctx;

/// The bus address of the pack data of slot `id` of `family`, or `None` when the image has no slot
/// (or has no packs): the directory is read through the machine's own memory ([`Ctx::peek8`]), so the
/// redirector sees the image the game sees.
#[must_use]
pub(super) fn overlay(ctx: &Ctx<'_>, family: Family, id: u16) -> Option<u32> {
    ssd_mod::find(|address| ctx.peek8(address), family, id)
}
