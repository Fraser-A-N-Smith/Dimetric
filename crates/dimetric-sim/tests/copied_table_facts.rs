//! Why neither runtime guard on a copied table can work.
//!
//! Kept as a test rather than a comment because both facts decide a design and
//! both are easy to misremember.

/// `__newindex` fires only for a key that is *absent*.
///
/// So a metatable on a populated table catches a brand-new field and lets a
/// write to an existing one straight through. `docs/ENGINE-GAPS.md` already
/// records this for `require`'s freeze, where the write worth stopping is the
/// one that overwrites something the module defines.
#[test]
fn newindex_only_fires_for_absent_keys() {
    let lua = mlua::Lua::new();
    lua.load(
        r#"
        local fired = {}
        local t = { existing = 1 }
        setmetatable(t, { __newindex = function(tbl, k, v) fired[#fired+1] = k end })
        t.brand_new = 2
        t.existing = 99
        results = table.concat(fired, ",") .. "|" .. tostring(t.existing)
        "#,
    )
    .exec()
    .unwrap();
    let out: String = lua.globals().get("results").unwrap();
    assert_eq!(
        out, "brand_new|99",
        "a guard on a populated table misses a write to a key that is there"
    );
}

/// And an empty proxy — the shape that *would* see every write — is invisible
/// to the host's own table walk.
///
/// `table_to_value` iterates with mlua's `pairs`, which is `lua_next` and
/// ignores `__pairs`. So converting a proxy back into an engine value yields
/// nothing: `self.pending = pending` would quietly empty the variable, which is
/// a far worse bug than the one being fixed.
#[test]
fn an_empty_proxy_converts_back_to_nothing() {
    let lua = mlua::Lua::new();
    let proxy: mlua::Table = lua
        .load(
            r#"
            local real = { a = 1, b = 2 }
            local proxy = setmetatable({}, {
                __index = real,
                __pairs = function() return next, real, nil end,
            })
            return proxy
            "#,
        )
        .eval()
        .unwrap();
    let seen = proxy.pairs::<mlua::Value, mlua::Value>().count();
    assert_eq!(
        seen, 0,
        "the host's walk sees a proxy's contents after all, so this argument is stale"
    );
}
