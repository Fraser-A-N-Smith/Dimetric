-- The arena: routes input, casts spells, spawns waves, resolves upgrades.
--
-- Combat and the run structure live here rather than being spread across
-- entities, so the order of resolution is explicit rather than emergent. Two
-- scripts resolving hits in whatever order the scene happened to be walked is
-- how a replay stops reproducing.
--
-- Nothing transient is authored in the scene. Enemies and projectiles are
-- spawned, which means a room is a wave rather than a file and a run is a
-- sequence of them.

local spells = require("scripts/spellbook.lua")

local HUNDRED = 100

-- The run: five rooms, each a wave. Enemies are placed on a ring around the
-- arena centre at whole-degree steps, so where they arrive is reproducible
-- without consuming a random number.
local ROOMS = {
  { { "skeleton", 2 } },
  { { "skeleton", 2 }, { "wraith", 1 } },
  { { "skeleton", 2 }, { "wraith", 2 } },
  { { "wraith", 3 }, { "brute", 1 } },
  { { "skeleton", 3 }, { "wraith", 2 }, { "brute", 2 } },
}

local CENTRE = vec2(fx.new(160), fx.new(96))
local SPAWN_RING = fx.new(70)

-- -- finding things ----------------------------------------------------

local function cached(self, key, path)
  local id = self[key]
  if id then
    local node = scene.by_id(id)
    if node then return node end
  end
  local found = scene.find(path)
  if found then self[key] = found:id() end
  return found
end

local function player(self)
  return cached(self, "player_id", "/Arena01/Player")
end

-- -- spell shaping -----------------------------------------------------

-- Percentages over a hundred, in integer arithmetic: a float here would be a
-- divergence on somebody else's machine three weeks from now.
local function apply_modifiers(spell, mods, definitions)
  local out = {}
  for k, v in pairs(spell) do out[k] = v end
  for i = 1, #mods do
    local mod = definitions[mods[i]]
    if mod then
      if mod.damage_pct then out.damage = (out.damage * mod.damage_pct) // HUNDRED end
      if mod.speed_pct then out.speed = (out.speed * mod.speed_pct) // HUNDRED end
      if mod.life_pct then out.life = (out.life * mod.life_pct) // HUNDRED end
      if mod.radius_pct then out.radius = (out.radius * mod.radius_pct) // HUNDRED end
      if mod.cooldown_pct then out.cooldown = (out.cooldown * mod.cooldown_pct) // HUNDRED end
      if mod.count_add then out.count = (out.count or 1) + mod.count_add end
      if mod.spread_add then out.spread = (out.spread or 0) + mod.spread_add end
    end
  end
  if out.cooldown < 4 then out.cooldown = 4 end
  if out.life < 1 then out.life = 1 end
  return out
end

-- The spell a run is casting: an evolution when two bases combine, otherwise
-- the first base held.
local function active_spell(self)
  local held = self.held or {}

  local spell
  if #held >= 2 then
    local a, c = held[1], held[2]
    if a > c then a, c = c, a end
    spell = spells.evolutions[a .. "+" .. c]
    self.evolved = spell and spell.name or ""
  end
  if not spell and #held >= 1 then
    spell = spells.base[held[1]]
    self.evolved = ""
  end
  if not spell then return nil end
  return apply_modifiers(spell, self.mods or {}, spells.modifiers)
end

-- -- casting -----------------------------------------------------------

local function launch(self, at, degrees, spell)
  local id = scene.spawn("prefabs/bolt", at)
  -- The id comes back before the node does, so the numbers it needs are queued
  -- alongside it and written when it arrives. A script cannot reach into a node
  -- that does not exist yet, and pretending otherwise would be the same
  -- ordering bug the deferral exists to prevent.
  local pending = self.pending or {}
  pending[#pending + 1] = {
    id = id,
    life = spell.life,
    damage = spell.damage,
    speed = spell.speed,
    homing = spell.homing or 0,
    orbit = spell.orbit or 0,
    angle = degrees,
  }
  self.pending = pending
  self.fired = (self.fired or 0) + 1
end

local function cast(self, p, spell)
  local origin = p.pos
  local base = tonumber(self.aim or "0") or 0
  local shape = spell.shape

  if shape == "single" then
    launch(self, origin, base, spell)

  elseif shape == "ring" then
    local count = spell.count or 8
    for i = 0, count - 1 do
      launch(self, origin, (360 * i) // count, spell)
    end

  elseif shape == "spread" then
    local count = spell.count or 3
    local spread = spell.spread or 12
    for i = 0, count - 1 do
      launch(self, origin, base + (i - (count - 1) // 2) * spread, spell)
    end

  elseif shape == "orbit" then
    local count = spell.count or 3
    for i = 0, count - 1 do
      launch(self, origin, (360 * i) // count, spell)
    end

  elseif shape == "seek" then
    local count = spell.count or 2
    for i = 0, count - 1 do
      launch(self, origin, (360 * i) // count, spell)
    end
  end
end

-- Hand the queued numbers to the projectiles that have now arrived.
local function arm_pending(self)
  local pending = self.pending
  if not pending or #pending == 0 then return end
  local still_waiting = {}
  for i = 1, #pending do
    local p = pending[i]
    local node = scene.by_id(p.id)
    if node then
      node.life = p.life
      node.damage = p.damage
      node.speed = p.speed
      node.homing = p.homing
      node.orbit_radius = p.orbit
      node.orbit_angle = p.angle
      if p.orbit > 0 then
        node:set_velocity(vec2(fx.new(0), fx.new(0)))
      else
        node:set_velocity(fx.from_angle(tostring(p.angle)) * fx.new(p.speed))
      end
    else
      still_waiting[#still_waiting + 1] = p
    end
  end
  self.pending = still_waiting
end

-- -- waves -------------------------------------------------------------

local function spawn_wave(self, room)
  local wave = ROOMS[room]
  if not wave then return 0 end
  local spawned = 0
  local slot = 0
  for i = 1, #wave do
    local kind, count = wave[i][1], wave[i][2]
    for _ = 1, count do
      -- Whole degrees around a ring: reproducible, and spread out enough that
      -- the wave does not arrive as one clump.
      local degrees = (slot * 47) % 360
      local at = CENTRE + fx.from_angle(tostring(degrees)) * SPAWN_RING
      scene.spawn("prefabs/" .. kind, at)
      slot = slot + 1
      spawned = spawned + 1
    end
  end
  return spawned
end

-- -- upgrades ----------------------------------------------------------

-- Offer three choices from the *upgrade* stream. A stream of its own, so
-- firing one fewer spell does not shift which upgrade a run is offered.
local function offer(self)
  local order, mods = spells.offer_order, spells.modifier_order
  local choices = {}
  for i = 1, 3 do
    if rng.chance("upgrade", 1, 2) then
      choices[i] = order[rng.range("upgrade", 1, #order)]
    else
      choices[i] = mods[rng.range("upgrade", 1, #mods)]
    end
  end
  self.offered = choices
end

local function pick(self, choice)
  if spells.base[choice] then
    local held = self.held or {}
    for i = 1, #held do
      if held[i] == choice then return end
    end
    held[#held + 1] = choice
    self.held = held
  else
    local mods = self.mods or {}
    mods[#mods + 1] = choice
    self.mods = mods
  end
end

-- -- lifecycle ---------------------------------------------------------

function on_ready(self)
  self.kills = 0
  self.fired = 0
  self.cooldown = 0
  self.room = 0
  self.rooms_total = #ROOMS
  self.held = { "bolt" }
  self.mods = {}
  self.evolved = ""
  self.pending = {}
  self.run_complete = false
  self.wave_pending = true
end

function on_tick(self)
  local p = player(self)
  if not p then return end

  arm_pending(self)

  -- The first wave, and every wave after an upgrade. Deferred to a tick rather
  -- than done in `on_ready`, because a spawn needs the scene to exist.
  if self.wave_pending then
    self.wave_pending = false
    self.room = (self.room or 0) + 1
    self.alive = spawn_wave(self, self.room)
    return
  end

  p.move = input.move()
  self.aim = input.aim()

  -- On the edge, not the hold: you clear a room with the fire button down, so
  -- an offer read from a held button is taken the instant it appears.
  if self.offered then
    if input.pressed("fire") then
      take_upgrade(self, 1)
    elseif input.pressed("alt") then
      take_upgrade(self, 2)
    elseif input.pressed("dash") then
      take_upgrade(self, 3)
    end
    return
  end

  if (self.cooldown or 0) > 0 then
    self.cooldown = self.cooldown - 1
  end

  local spell = active_spell(self)
  if spell and input.held("fire") and (self.cooldown or 0) <= 0 then
    cast(self, p, spell)
    self.cooldown = spell.cooldown
  end

  -- Orbiting spells follow the caster rather than flying, so they are moved
  -- here where the player's position is already in hand.
  if spell and (spell.orbit or 0) > 0 then
    local orbiting = scene.near(p.pos, fx.new(64), "projectile")
    for i = 1, #orbiting do
      local node = orbiting[i]
      if (node.orbit_radius or 0) > 0 then
        local turned = ((node.orbit_angle or 0) + 3) % 360
        node.orbit_angle = turned
        node.pos = p.pos + fx.from_angle(tostring(turned)) * fx.new(node.orbit_radius)
      end
    end
  end
end

function on_post_tick(self)
  local enemies = scene.tagged("enemy")
  self.enemy_count = #enemies
  self.live_projectiles = #scene.tagged("projectile")

  -- Offer once, on the transition to an empty room. Offering whenever the room
  -- happens to be empty means a held button takes an upgrade every tick.
  if self.enemy_count > 0 then
    self.cleared = false
  elseif not self.cleared and not self.run_complete and not self.wave_pending then
    self.cleared = true
    if (self.room or 0) >= #ROOMS then
      self.run_complete = true
    else
      offer(self)
    end
  end
end

-- Called by an enemy as it dies.
function note_kill(self)
  self.kills = (self.kills or 0) + 1
end

-- Take one of the three offers and move to the next room.
function take_upgrade(self, index)
  local offered = self.offered
  if not offered or not offered[index] then return end
  pick(self, offered[index])
  self.offered = nil
  self.cleared = false
  self.wave_pending = true
end
