-- The arena: routes input, casts spells, resolves upgrades, tracks the run.
--
-- Combat lives here rather than being spread across entities so that the order
-- of resolution is explicit rather than emergent. Two scripts resolving hits in
-- whatever order the scene happened to walk is how a replay stops reproducing.
--
-- # Projectiles are pooled
--
-- The script API has no way to create a node, so projectiles are authored in
-- the scene up front, parked, and taken from a free list when a spell is cast.
-- That is a workaround for an engine gap, not a design — though pooling is what
-- a game at this sprite density would do regardless. See `docs/ENGINE-GAPS.md`.

local PARKED = vec2(fx.new(-9000), fx.new(-9000))
local HUNDRED = fx.new(100)

-- -- finding things ----------------------------------------------------
--
-- Paths are resolved once and cached as ids. `scene.find` walks the sibling
-- list matching each name, which is ~3.9us against 140ns for a handle
-- validation; doing it every tick for every projectile would dominate the frame.

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

local function book(self)
  return cached(self, "book_id", "/Arena01/Spellbook")
end

local function player(self)
  return cached(self, "player_id", "/Arena01/Player")
end

-- -- spell shaping -----------------------------------------------------

-- Apply the modifiers a run has picked up. Percentages over a hundred, in
-- integer arithmetic: a float here would be a divergence on somebody else's
-- machine three weeks from now.
local function apply_modifiers(spell, mods, definitions)
  local out = {}
  for k, v in pairs(spell) do out[k] = v end
  for i = 1, #mods do
    local mod = definitions[mods[i]]
    if mod then
      if mod.damage_pct then out.damage = (out.damage * mod.damage_pct) // 100 end
      if mod.speed_pct then out.speed = (out.speed * mod.speed_pct) // 100 end
      if mod.life_pct then out.life = (out.life * mod.life_pct) // 100 end
      if mod.radius_pct then out.radius = (out.radius * mod.radius_pct) // 100 end
      if mod.cooldown_pct then out.cooldown = (out.cooldown * mod.cooldown_pct) // 100 end
      if mod.count_add then out.count = (out.count or 1) + mod.count_add end
      if mod.spread_add then out.spread = (out.spread or 0) + mod.spread_add end
    end
  end
  if out.cooldown < 4 then out.cooldown = 4 end
  if out.life < 1 then out.life = 1 end
  return out
end

-- The spell a run is actually casting: an evolution when two bases combine,
-- otherwise the first base held.
local function active_spell(self)
  local b = book(self)
  if not b then return nil end
  local held = self.held or {}
  local definitions = b.base
  if not definitions then return nil end

  local spell
  if #held >= 2 then
    local a, c = held[1], held[2]
    if a > c then a, c = c, a end
    spell = b.evolutions[a .. "+" .. c]
    self.evolved = spell and spell.name or ""
  end
  if not spell and #held >= 1 then
    spell = definitions[held[1]]
    self.evolved = ""
  end
  if not spell then return nil end
  return apply_modifiers(spell, self.mods or {}, b.modifiers)
end

-- -- the projectile pool -----------------------------------------------

local function pool(self)
  local ids = self.pool_ids
  if ids then return ids end

  local found = {}
  local projectiles = scene.tagged("projectile")
  for i = 1, #projectiles do
    found[i] = projectiles[i]:id()
  end
  self.pool_ids = found
  self.pool_size = #found
  return found
end

-- Take an inactive projectile, or nothing when the pool is exhausted.
--
-- Exhaustion is a dropped shot rather than an error: a pool that grew would
-- allocate mid-tick, and a game that errored would stop on a busy screen.
local function take(self)
  local ids = pool(self)
  local start = (self.next_slot or 0)
  for offset = 0, #ids - 1 do
    local index = ((start + offset) % #ids) + 1
    local node = scene.by_id(ids[index])
    if node and node.life == nil or (node and (node.life or 0) <= 0) then
      self.next_slot = (start + offset + 1) % #ids
      return node
    end
  end
  self.dropped = (self.dropped or 0) + 1
  return nil
end

local function launch(self, node, at, direction, spell, orbit_angle)
  node.pos = at
  node.life = spell.life
  node.damage = spell.damage
  node.owner = "player"
  node.visible = true
  node.orbit_angle = orbit_angle or 0
  node.orbit_radius = spell.orbit or 0
  node.homing = spell.homing or 0
  node.speed = spell.speed
  node:set_velocity(direction * fx.new(spell.speed))
  self.fired = (self.fired or 0) + 1
end

-- -- casting -----------------------------------------------------------

local function cast(self, p, spell)
  local aim = fx.from_angle(self.aim or "0.0")
  local origin = p.pos
  local shape = spell.shape

  if shape == "single" then
    local node = take(self)
    if node then launch(self, node, origin, aim, spell) end

  elseif shape == "ring" then
    local count = spell.count or 8
    for i = 0, count - 1 do
      local node = take(self)
      if node then
        -- Degrees as text, because `fx.from_angle` parses an exact decimal and
        -- the trig tables are indexed by binary angle. No float ever appears.
        local degrees = tostring((360 * i) // count)
        launch(self, node, origin, fx.from_angle(degrees), spell)
      end
    end

  elseif shape == "spread" then
    local count = spell.count or 3
    local spread = spell.spread or 12
    local base = tonumber(self.aim or "0") or 0
    for i = 0, count - 1 do
      local node = take(self)
      if node then
        local offset = (i - (count - 1) // 2) * spread
        launch(self, node, origin, fx.from_angle(tostring(base + offset)), spell)
      end
    end

  elseif shape == "orbit" then
    local count = spell.count or 3
    for i = 0, count - 1 do
      local node = take(self)
      if node then
        local degrees = (360 * i) // count
        launch(self, node, origin, vec2(fx.new(0), fx.new(0)), spell, degrees)
      end
    end

  elseif shape == "seek" then
    local count = spell.count or 2
    for i = 0, count - 1 do
      local node = take(self)
      if node then
        local degrees = tostring((360 * i) // count)
        launch(self, node, origin, fx.from_angle(degrees), spell)
      end
    end
  end
end

-- -- upgrades ----------------------------------------------------------

-- Offer three choices, drawn from the *upgrade* stream.
--
-- A stream of its own, so that firing one fewer spell does not shift which
-- upgrade a run is offered. Sharing the stream with anything else would make
-- the whole run's build depend on how many projectiles happened to spawn.
local function offer(self)
  local b = book(self)
  if not b then return end
  local order = b.offer_order
  local mods = b.modifier_order
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
  local b = book(self)
  if not b then return end
  if b.base[choice] then
    local held = self.held or {}
    -- Holding the same base twice is a no-op; two distinct ones evolve.
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
  self.dropped = 0
  self.cooldown = 0
  self.room = 1
  self.held = { "bolt" }
  self.mods = {}
  self.evolved = ""
  self.cleared = false
  self.run_complete = false
  pool(self)
end

function on_tick(self)
  local p = player(self)
  if not p then return end

  -- Edge detection, kept here because the script API exposes `input.held` and
  -- not `input.pressed`: a button is compared against what it was last tick.
  -- Cheap to do, and the reason it matters is below.
  local fire, alt, dash = input.held("fire"), input.held("alt"), input.held("dash")
  local fire_edge = fire and not self.fire_was
  local alt_edge = alt and not self.alt_was
  local dash_edge = dash and not self.dash_was
  self.fire_was, self.alt_was, self.dash_was = fire, alt, dash

  -- While an upgrade is offered the run is paused on the choice. The three
  -- buttons pick the three offers, which keeps the whole loop — clear, choose,
  -- evolve — inside an input log and therefore inside a replay.
  --
  -- On the edge rather than the hold: you clear a room with the fire button
  -- down, so an offer read from a held button is taken the instant it appears
  -- and you never see it.
  if self.offered then
    if fire_edge then
      take_upgrade(self, 1)
    elseif alt_edge then
      take_upgrade(self, 2)
    elseif dash_edge then
      take_upgrade(self, 3)
    end
    return
  end

  -- Input reaches the player through the arena, so the player script never
  -- touches a raw device and a replay only has to reproduce this one routing.
  p.move = input.move()
  self.aim = input.aim()

  if (self.cooldown or 0) > 0 then
    self.cooldown = self.cooldown - 1
  end

  local spell = active_spell(self)
  if spell and fire and (self.cooldown or 0) <= 0 then
    cast(self, p, spell)
    self.cooldown = spell.cooldown
  end

  -- Orbiting projectiles follow the caster rather than flying, so they are
  -- moved here where the player's position is already in hand.
  if spell and (spell.orbit or 0) > 0 then
    local ids = pool(self)
    for i = 1, #ids do
      local node = scene.by_id(ids[i])
      if node and (node.life or 0) > 0 and (node.orbit_radius or 0) > 0 then
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
  local live = 0
  local ids = pool(self)
  for i = 1, #ids do
    local node = scene.by_id(ids[i])
    if node and (node.life or 0) > 0 then live = live + 1 end
  end
  self.live_projectiles = live

  -- Offer once, on the transition to an empty room. Offering whenever the room
  -- happens to be empty means a held button takes an upgrade every tick, which
  -- is exactly what it did the first time this was written.
  if self.enemy_count > 0 then
    self.cleared = false
  elseif not self.cleared then
    self.cleared = true
    if not self.run_complete then
      offer(self)
    end
  end
end

-- Called by a projectile when it kills something.
function note_kill(self)
  self.kills = (self.kills or 0) + 1
end

-- Called by the run driver, or by a test, to take an offered upgrade.
function take_upgrade(self, index)
  local offered = self.offered
  if not offered or not offered[index] then return end
  pick(self, offered[index])
  self.offered = nil
  self.room = (self.room or 1) + 1

  -- One room per scene. Moving to the next one means loading another scene,
  -- and a script cannot do that any more than it can create a node — so the
  -- run ends here and the room graph is driven from outside. See
  -- `docs/ENGINE-GAPS.md`.
  self.run_complete = true
end
