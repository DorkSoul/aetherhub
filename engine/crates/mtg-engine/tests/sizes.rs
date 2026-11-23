//! Guards the in-memory size of the card DSL. The whole card database holds one `Effect`/`Ability` per effect/ability
//! of ~65k cards, so a fat enum variant multiplies into gigabytes (an inline `Filter` would make `Effect` 21 KB and the
//! database 8-11 GB; the limits sit just above the current sizes, so growth shows up here). If this fails, box the new big field (`Box<T>` / `FilterBox`) instead of raising the limit.
use mtg_engine::dsl::*;
use std::mem::size_of;

#[test]
fn dsl_types_stay_small() {
    let sizes = [
        ("Effect", size_of::<Effect>(), 1400),
        ("Ability", size_of::<Ability>(), 1800),
        ("Condition", size_of::<Condition>(), 384),
        ("Trigger", size_of::<Trigger>(), 384),
        ("Cost", size_of::<Cost>(), 256),
        ("Amount", size_of::<Amount>(), 192),
        ("Sel", size_of::<Sel>(), 64),
        ("TargetSpec", size_of::<TargetSpec>(), 640),
        ("StaticEffect", size_of::<StaticEffect>(), 512),
        ("Replace", size_of::<Replace>(), 1400),
        ("CardDef", size_of::<CardDef>(), 512),
    ];
    let mut bad = vec![];
    for (n, got, max) in sizes {
        println!("{n}: {got} bytes (limit {max})");
        if got > max {
            bad.push(format!("{n} is {got} bytes, limit {max}"));
        }
    }
    assert!(bad.is_empty(), "DSL types grew: {bad:?}");
}
