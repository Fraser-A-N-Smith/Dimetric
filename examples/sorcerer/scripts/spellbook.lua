-- Every spell in the game, as data.
--
-- Balance tuning is editing this file and reloading: `dim run --watch` picks it
-- up at the next tick boundary, and the node variables the arena reads are
-- rebuilt from `on_ready`. Nothing here is compiled into the engine.
--
-- Why a node rather than a module: the script sandbox has no `require`, so two
-- scripts cannot share a table directly. A node can publish one, because script
-- variables are engine state and any script can read another node's. See
-- `docs/ENGINE-GAPS.md` — this is the workaround, not the intended shape.

-- Base spells. `shape` is what the arena does with a cast; everything else is
-- numbers the projectile carries.
local BASE = {
  bolt = {
    name = "Bolt",
    shape = "single",
    damage = 12,
    speed = 150,
    life = 48,
    radius = 3,
    cooldown = 12,
    tint = "#8fd4ff",
  },
  nova = {
    name = "Nova",
    shape = "ring",
    count = 8,
    damage = 7,
    speed = 90,
    life = 26,
    radius = 4,
    cooldown = 30,
    tint = "#ffd48f",
  },
  chain = {
    name = "Chain",
    shape = "spread",
    count = 3,
    spread = 14,
    damage = 8,
    speed = 130,
    life = 40,
    radius = 3,
    cooldown = 18,
    tint = "#c58fff",
  },
  ward = {
    name = "Ward",
    shape = "orbit",
    count = 3,
    damage = 6,
    speed = 0,
    life = 240,
    radius = 5,
    cooldown = 120,
    orbit = 26,
    tint = "#8fffc4",
  },
  summon = {
    name = "Summon",
    shape = "seek",
    count = 2,
    damage = 10,
    speed = 70,
    life = 180,
    radius = 4,
    cooldown = 90,
    homing = 12,
    tint = "#ff8f8f",
  },
}

-- Modifiers multiply or add to whatever they are applied to. Kept as whole
-- numbers over a denominator of 100, because a modifier is gameplay and a
-- float would be a replay divergence waiting for a different machine.
local MODIFIERS = {
  fierce = { name = "Fierce", damage_pct = 150 },
  swift = { name = "Swift", speed_pct = 140, cooldown_pct = 70 },
  wide = { name = "Wide", count_add = 2, spread_add = 6 },
  lasting = { name = "Lasting", life_pct = 200 },
  heavy = { name = "Heavy", radius_pct = 180, damage_pct = 120, speed_pct = 70 },
}

-- Two spells held at once evolve into a third. The pair is order-independent;
-- the arena looks up "a+b" with the names sorted.
local EVOLUTIONS = {
  ["bolt+chain"] = {
    name = "Forking Arc",
    shape = "spread",
    count = 5,
    spread = 10,
    damage = 14,
    speed = 160,
    life = 44,
    radius = 3,
    cooldown = 16,
    tint = "#b8f0ff",
  },
  ["nova+ward"] = {
    name = "Pulsing Barrier",
    shape = "orbit",
    count = 6,
    damage = 11,
    speed = 0,
    life = 300,
    radius = 6,
    cooldown = 150,
    orbit = 30,
    tint = "#b8ffe0",
  },
  ["bolt+nova"] = {
    name = "Starfall",
    shape = "ring",
    count = 12,
    damage = 10,
    speed = 110,
    life = 34,
    radius = 4,
    cooldown = 34,
    tint = "#ffe9b8",
  },
  ["chain+summon"] = {
    name = "Hunting Swarm",
    shape = "seek",
    count = 4,
    damage = 9,
    speed = 95,
    life = 200,
    radius = 3,
    cooldown = 80,
    homing = 18,
    tint = "#ffb8d4",
  },
  ["summon+ward"] = {
    name = "Sentinel Ring",
    shape = "orbit",
    count = 4,
    damage = 13,
    speed = 0,
    life = 320,
    radius = 6,
    cooldown = 160,
    orbit = 20,
    tint = "#c4ffb8",
  },
  ["bolt+ward"] = {
    name = "Lance",
    shape = "single",
    damage = 30,
    speed = 200,
    life = 60,
    radius = 4,
    cooldown = 26,
    tint = "#dfefff",
  },
  ["chain+nova"] = {
    name = "Scatterburst",
    shape = "ring",
    count = 10,
    damage = 8,
    speed = 120,
    life = 30,
    radius = 3,
    cooldown = 28,
    tint = "#e2c4ff",
  },
  ["bolt+summon"] = {
    name = "Seeking Bolt",
    shape = "seek",
    count = 2,
    damage = 16,
    speed = 120,
    life = 120,
    radius = 3,
    cooldown = 40,
    homing = 20,
    tint = "#ffc4b8",
  },
  ["chain+ward"] = {
    name = "Thorn Field",
    shape = "orbit",
    count = 5,
    damage = 9,
    speed = 0,
    life = 280,
    radius = 4,
    cooldown = 130,
    orbit = 16,
    tint = "#d4ffb8",
  },
  ["nova+summon"] = {
    name = "Detonating Swarm",
    shape = "ring",
    count = 9,
    damage = 12,
    speed = 100,
    life = 40,
    radius = 5,
    cooldown = 70,
    homing = 8,
    tint = "#ffd4c4",
  },
}

-- The order upgrades are offered in. A list rather than the table's own key
-- order, because iterating a Lua table is unordered and an unordered upgrade
-- roll is a replay that diverges (invariant I4).
local OFFER_ORDER = { "bolt", "nova", "chain", "ward", "summon" }
local MODIFIER_ORDER = { "fierce", "swift", "wide", "lasting", "heavy" }

function on_ready(self)
  self.base = BASE
  self.modifiers = MODIFIERS
  self.evolutions = EVOLUTIONS
  self.offer_order = OFFER_ORDER
  self.modifier_order = MODIFIER_ORDER
  self.spell_count = #OFFER_ORDER
end
