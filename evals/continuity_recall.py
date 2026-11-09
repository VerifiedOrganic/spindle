#!/usr/bin/env python3
"""Continuity detection recall & false-positive benchmark over MCP stdio.

Builds two non-trivial 20-chapter synthetic manuscripts (`ashfall_expedition` and
`veridian_station`) against a disposable Spindle MCP server/data directory. For
each manuscript, runs `check_consistency` on:
  1. An unplanted control copy (20 distinct chapters, clean canon) to measure
     false-positive count by `check_type`.
  2. A planted copy with N=14 contradictions (2 per known contradiction type,
     28 total across both manuscripts) spanning all 7 audited categories:
       - `character_attribute`
       - `dead_character_acting`
       - `timeline_chronology_rewind`
       - `knowledge_leak_before_learned`
       - `world_rule_cost_violation`
       - `quantity_affordability`
       - `secret_leak`

Each category includes both a lexical/structured contradiction (matching the
deterministic validators' documented surface) and a natural-prose contradiction
(paraphrased or un-keyed prose drift) so deterministic recall is measured
honestly rather than artificially inflated.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[1]

CONTRADICTION_TYPES: tuple[str, ...] = (
    "character_attribute",
    "dead_character_acting",
    "timeline_chronology_rewind",
    "knowledge_leak_before_learned",
    "world_rule_cost_violation",
    "quantity_affordability",
    "secret_leak",
)


@dataclass(frozen=True)
class PlantedContradiction:
    contradiction_id: str
    contradiction_type: str
    variant: str
    chapter_number: int
    description: str
    expected_check_types: tuple[str, ...]
    match_tokens: tuple[str, ...]


def resolve_agent_config(
    agent_config: Path | None,
    environ: dict[str, str] | None = None,
) -> Path | None:
    if agent_config is not None:
        return agent_config.expanduser().resolve()
    env = os.environ if environ is None else environ
    raw = env.get("SPINDLE_AGENT_CONFIG") or env.get("SPINDLE_CONFIG")
    if raw and raw.strip():
        return Path(raw.strip()).expanduser().resolve()
    return None


class McpClient:
    """Line-delimited JSON-RPC 2.0 MCP client over a disposable server process."""

    @staticmethod
    def build_env(
        data_dir: Path,
        agent_config: Path | None = None,
        base_env: dict[str, str] | None = None,
    ) -> dict[str, str]:
        env = (os.environ if base_env is None else base_env).copy()
        env["SPINDLE_DATA_DIR"] = str(data_dir)
        # The benchmark seeds calendars, locations, quantities and canon through
        # the canonical authoring surface; R+1's unset default is the narrower
        # workflow-write profile.
        env["SPINDLE_TOOL_PROFILE"] = "workflow-authoring"
        env.pop("SPINDLE_HTTP_ADDR", None)
        env.pop("SPINDLE_CONFIG", None)
        env.pop("SPINDLE_AGENT_CONFIG", None)
        if agent_config is not None:
            env["SPINDLE_CONFIG"] = str(agent_config.expanduser().resolve())
        return env

    def __init__(self, binary: Path, data_dir: Path, agent_config: Path | None = None) -> None:
        env = self.build_env(data_dir, agent_config=agent_config)
        self.proc = subprocess.Popen(
            [str(binary)],
            cwd=str(data_dir),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
            env=env,
        )
        self.next_id = 0
        self.request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "spindle-continuity-recall-eval", "version": "0.1.0"},
            },
        )
        self.notify("notifications/initialized", {})

    def notify(self, method: str, params: dict[str, Any]) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(
            json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n"
        )
        self.proc.stdin.flush()

    def request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        assert self.proc.stdin is not None and self.proc.stdout is not None
        self.next_id += 1
        req_id = self.next_id
        self.proc.stdin.write(
            json.dumps(
                {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
            )
            + "\n"
        )
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"MCP server closed stdout while waiting for {method}")
            message = json.loads(line)
            if message.get("id") == req_id:
                if "error" in message:
                    raise RuntimeError(f"{method} failed: {message['error']}")
                return message["result"]

    def tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        text = "".join(
            block.get("text", "")
            for block in result.get("content", [])
            if block.get("type") == "text"
        )
        if result.get("isError"):
            raise RuntimeError(f"{name} returned tool error: {text}")
        return json.loads(text) if text else {}

    def close(self) -> None:
        if self.proc.stdin:
            self.proc.stdin.close()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def resolve_binary(explicit: str | None, no_build: bool = False) -> Path:
    if explicit:
        path = Path(explicit).expanduser().resolve()
        if not path.is_file():
            raise FileNotFoundError(f"spindle-mcp binary not found at {path}")
        return path
    if not no_build:
        # Cargo knows the executable's actual dependency graph, including build
        # scripts/assets. Timestamp scans also catch non-build/test-only files
        # that Cargo rightly leaves out, and can permanently reject a fresh exe.
        built = subprocess.run(
            ["cargo", "build", "--quiet", "-p", "spindle-mcp", "--message-format=json"],
            cwd=ROOT,
            check=True,
            stdout=subprocess.PIPE,
            text=True,
        )
        for line in built.stdout.splitlines():
            artifact = json.loads(line)
            target = artifact.get("target", {})
            executable = artifact.get("executable")
            if (artifact.get("reason") == "compiler-artifact"
                    and target.get("name") == "spindle-mcp"
                    and "bin" in target.get("kind", []) and executable):
                candidate = Path(executable).resolve()
                if not candidate.is_file():
                    raise FileNotFoundError(f"Cargo executable not found at {candidate}")
                return candidate
        raise RuntimeError("Cargo did not report a spindle-mcp executable; no stale fallback used")
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = ROOT / target
    profiles = ("debug", "release") if no_build else ("debug",)
    for profile in profiles:
        candidate = target / profile / "spindle-mcp"
        if candidate.is_file():
            return candidate
    raise FileNotFoundError(
        "spindle-mcp binary not found; build it or pass --binary explicitly"
    )


# ---------------------------------------------------------------------------
# Manuscript 1: The Ashfall Expedition (20 chapters, dark fantasy siege)
# ---------------------------------------------------------------------------

ASHFALL_CHAPTERS: list[dict[str, Any]] = [
    {
        "chapter": 1,
        "location": "Ash Gate Bastion",
        "day": 1,
        "summary": "Captain Mara Vance inspects the Ash Gate defenses and tallies the garrison reserves.",
        "prose": (
            "Captain Mara Vance walked the slick parapet of the Ash Gate Bastion while sleet hissed "
            "against the braziers. Below the battlements, Quartermaster Bram Thorne counted crates of "
            "salted mutton and lamp oil beside the windlass.\n\n"
            "\"Hold the north embrasure clear of slag,\" Mara Vance said, resting her iron-shod palm "
            "on the cold parapet. \"If the ravine floods before moonrise, the lower sluice will jam.\"\n\n"
            "Archivist Oren Vale unrolled his wax-sealed ledger beneath the watch-lantern and marked "
            "the garrison's winter ration count."
        ),
    },
    {
        "chapter": 2,
        "location": "Sunken Cistern",
        "day": 2,
        "summary": "Scout Lyra Voss and Mara Vance inspect the Sunken Cistern drainage grates.",
        "prose": (
            "Water dripped from the vaulted ribs of the Sunken Cistern as Scout Lyra Voss tested the "
            "iron chains with a crowbar. Mara Vance held a hooded lantern over the black pool where "
            "blind cave-carp circled the intake grate.\n\n"
            "\"The mortar along the eastern culvert is still sound,\" Lyra Voss said. \"No sapper has "
            "touched these stones since the autumn thaw.\"\n\n"
            "They bolted the bronze overflow hatch and climbed the spiral stair back toward the courtyard."
        ),
    },
    {
        "chapter": 3,
        "location": "Hollow Market",
        "day": 3,
        "summary": "Quartermaster Bram Thorne negotiates grain and timber tariffs in the Hollow Market.",
        "prose": (
            "Beneath the canvas awnings of the Hollow Market, Quartermaster Bram Thorne weighed sacks "
            "of kiln-dried barley against brass counterweights. Drovers argued over mule fodder while "
            "rain drummed on the slate eaves.\n\n"
            "\"Ten copper bits for the cordwood and not a farthing more,\" Bram Thorne told the carter, "
            "knotting his leather drawstring pouch. Envoy Caelen Dravos watched the exchange from the "
            "archway, his travel cloak dusted with grey road-salt."
        ),
    },
    {
        "chapter": 4,
        "location": "Whispering Archive",
        "day": 4,
        "summary": "Archivist Oren Vale catalogs pre-siege treaties in the Whispering Archive.",
        "prose": (
            "Tall tallow candles burned in brass sconces along the Whispering Archive while Archivist "
            "Oren Vale brushed volcanic grit from a cedar scroll-case. Mara Vance leaned over the oak "
            "lectern to study the boundary survey of the Red March.\n\n"
            "\"Every surveyor's seal from the third regency is intact,\" Oren Vale murmured, turning "
            "the vellum leaf with ivory tongs. \"The mine shafts beneath the ridge were bricked shut "
            "sixty winters ago.\""
        ),
    },
    {
        "chapter": 5,
        "location": "Obsidian Causeway",
        "day": 5,
        "summary": "Bram Thorne and Lyra Voss escort the supply wagons across the Obsidian Causeway.",
        "prose": (
            "Wind scoured the black basalt slabs of the Obsidian Causeway as Quartermaster Bram Thorne "
            "walked beside the lead ox-wain. Scout Lyra Voss ranged thirty paces ahead, scanning the "
            "shale scree for rockslide fissures.\n\n"
            "\"Keep the rear axle chained until we clear the switchback,\" Bram Thorne called over the "
            "creak of the timber yoke. The drivers eased the heavy caissons past the mile-marker stone."
        ),
    },
    {
        "chapter": 6,
        "location": "Riven Bridge",
        "day": 6,
        "summary": "An ambush at Riven Bridge claims Scout Lyra Voss while Caelen Dravos retreats.",
        "prose": (
            "Crossbow quarrels rattled off the broken parapet of Riven Bridge as Commander Varek Sol's "
            "outriders charged from the fog. Scout Lyra Voss held the narrow keystone arch long enough "
            "for Envoy Caelen Dravos to drop the portcullis chain, taking a steel lance through the "
            "ribs before the gate crashed shut.\n\n"
            "Mara Vance dragged Lyra Voss behind the merlon, but the scout's pulse had already stilled. "
            "By nightfall the garrison wrapped Lyra Voss in a grey watch-cloak and laid her under the "
            "cairn beside the bridge tower."
        ),
    },
    {
        "chapter": 7,
        "location": "Ember Foundry",
        "day": 7,
        "summary": "Oren Vale and Mara Vance oversee the recasting of siege chains in the Ember Foundry.",
        "prose": (
            "Sparks showered from the tilt-hammers of the Ember Foundry as smiths poured white-hot slag "
            "into sand molds. Mara Vance stood by the quenching trough while Archivist Oren Vale checked "
            "the alloy temper against his tables.\n\n"
            "\"Three parts bog-iron to one part mountain ore,\" Oren Vale said, shielding his eyes "
            "from the glare. \"The new drawbridge links will hold against a battering ram.\""
        ),
    },
    {
        "chapter": 8,
        "location": "Spire Watchtower",
        "day": 8,
        "summary": "Mara Vance and Bram Thorne track signal fires from the Spire Watchtower.",
        "prose": (
            "From the windy gallery of the Spire Watchtower, Mara Vance counted three pitch beacons "
            "burning along the southern escarpment. Quartermaster Bram Thorne adjusted the brass "
            "azimuth ring and logged the bearing in the night watchbook.\n\n"
            "\"Varek Sol is moving his siege engines toward the dry moat,\" Mara Vance said. \"Double "
            "the bucket brigades on the lower curtain wall before morning.\""
        ),
    },
    {
        "chapter": 9,
        "location": "Glass Reliquary",
        "day": 9,
        "summary": "Mara Vance channels soulforge fire at the Glass Reliquary at a heavy blood price.",
        "prose": (
            "Inside the Glass Reliquary, Mara Vance drew her utility blade across her palm and let "
            "three drops of blood hiss onto the iron anvil before daring to channel the soulforge "
            "ember. Rime crept up her knuckles as the pale flame caught, numbing her fingers to the "
            "second joint.\n\n"
            "Archivist Oren Vale wrapped her frostbitten hand in wool as soon as the seal fused shut. "
            "\"One more conduit like that and you will lose the use of your sword-hand for a week,\" "
            "Oren Vale warned."
        ),
    },
    {
        "chapter": 10,
        "location": "Hollow Market",
        "day": 10,
        "summary": "Bram Thorne purchases lantern wicks and tallow in the Hollow Market.",
        "prose": (
            "Quartermaster Bram Thorne returned to the chandler's stall in the Hollow Market as the "
            "curfew bell rang. He counted out 5 copper coins for a bundle of braided hemp wicks and "
            "two cakes of rendered mutton tallow.\n\n"
            "\"That leaves my purse light, but the watch-lanterns will burn through the fog,\" Bram "
            "Thorne told the chandler, tucking the receipt into his belt."
        ),
    },
    {
        "chapter": 11,
        "location": "Obsidian Causeway",
        "day": 11,
        "summary": "Mara Vance and Oren Vale inspect the barricades on the Obsidian Causeway.",
        "prose": (
            "Frost coated the gabions along the Obsidian Causeway as Mara Vance and Archivist Oren "
            "Vale walked the outer palisade. Sappers had wedged sharpened stakes between the basalt "
            "boulders to slow any night assault.\n\n"
            "\"The western approach is narrow enough for twenty pikemen to hold,\" Mara Vance observed, "
            "testing the lashings on a timber cheval-de-frise."
        ),
    },
    {
        "chapter": 12,
        "location": "Ash Gate Bastion",
        "day": 12,
        "summary": "Bram Thorne audits the bastion strongbox and distributes modest hazard pay.",
        "prose": (
            "In the vaulted pay-room of the Ash Gate Bastion, Quartermaster Bram Thorne balanced the "
            "iron strongbox beside a tallow dip. He stacked the remaining copper bits into neat rows "
            "and signed the muster roll.\n\n"
            "\"Every engineer on the ditch crew gets two coppers for boot-leather,\" Bram Thorne "
            "said, locking the strongbox hasp."
        ),
    },
    {
        "chapter": 13,
        "location": "Salt Spire",
        "day": 13,
        "summary": "Oren Vale and two choristers use a tuning fork to open the outer Salt Spire ward.",
        "prose": (
            "At the base of the Salt Spire, Archivist Oren Vale struck a silver tuning fork against "
            "the threshold stone while two choristers held the harmonic third. Frost bloomed across "
            "their sleeves as the ward lattice slowly yielded.\n\n"
            "\"Hold the pitch steady until the keystone clicks,\" Oren Vale gasped, his breath pluming "
            "white in the bitter draft."
        ),
    },
    {
        "chapter": 14,
        "location": "Whispering Archive",
        "day": 14,
        "summary": "Mara Vance and Bram Thorne review the muster rolls in the Whispering Archive.",
        "prose": (
            "Lantern light flickered across the map table in the Whispering Archive while Mara Vance "
            "compared the casualty lists against the ward rosters. Quartermaster Bram Thorne stood by "
            "the coal brazier, warming his hands.\n\n"
            "\"We have enough ash-bows for the east gallery, Captain,\" Bram Thorne said. \"The fletchers "
            "finished three hundred shafts before supper.\""
        ),
    },
    {
        "chapter": 15,
        "location": "Sunken Cistern",
        "day": 15,
        "summary": "Oren Vale reveals the Sunken Cistern gate cipher to Bram Thorne.",
        "prose": (
            "Beside the lower sluice of the Sunken Cistern, Archivist Oren Vale pried open the lead "
            "cylinder recovered from the old engineer's crypt and showed the engraved plate to "
            "Quartermaster Bram Thorne.\n\n"
            "\"Read it carefully, Bram: the cistern gate cipher is seven-ash-nine,\" Oren Vale said. "
            "Bram Thorne repeated the sequence under his breath and committed the tumblers to memory."
        ),
    },
    {
        "chapter": 16,
        "location": "Salt Spire",
        "day": 16,
        "summary": "Caelen Dravos discovers evidence that Varek Sol poisoned the Salt Spire well.",
        "prose": (
            "In the cellar beneath the Salt Spire, Envoy Caelen Dravos lifted a cracked stoneware jar "
            "from the cistern lip and smelled bitter nightshade oil. Mara Vance knelt beside the "
            "runoff channel with a tarnished silver spoon.\n\n"
            "\"Look at the residue on the flagstones—Commander Varek poisoned the well at Salt Spire,\" "
            "Caelen Dravos said grimly."
        ),
    },
    {
        "chapter": 17,
        "location": "Ember Foundry",
        "day": 17,
        "summary": "Caelen Dravos and Oren Vale inspect the counterweight winches in the Ember Foundry.",
        "prose": (
            "Chain hoists clanked in the rafters of the Ember Foundry as Envoy Caelen Dravos helped "
            "Archivist Oren Vale measure the torsion cables. Soot stained their cuffs while the "
            "apprentices greased the bronze bearings.\n\n"
            "\"These capstans can lift the inner portcullis even if the main counterweights crack,\" "
            "Caelen Dravos said, wiping axle-grease from his knuckles."
        ),
    },
    {
        "chapter": 18,
        "location": "Spire Watchtower",
        "day": 18,
        "summary": "Mara Vance watches the siege lines from the Spire Watchtower through the afternoon.",
        "prose": (
            "At noon, Mara Vance climbed to the upper deck of the Spire Watchtower and studied the "
            "entrenchments through a brass spyglass. Sappers were hauling fascines across the dry "
            "ditch.\n\n"
            "Two hours later, as afternoon shadows lengthened across the courtyard, she sent a runner "
            "down the stair to alert the crossbow companies."
        ),
    },
    {
        "chapter": 19,
        "location": "Riven Bridge",
        "day": 19,
        "summary": "Mara Vance and Bram Thorne repel a probing attack at Riven Bridge.",
        "prose": (
            "Grappling hooks clanged against the ironwork of Riven Bridge as Commander Varek Sol's "
            "vanguard tested the outer barbican. Mara Vance directed the pitch-cauldrons while "
            "Quartermaster Bram Thorne fed fresh quivers to the marksmen.\n\n"
            "\"Cut the scaling ladders on the left flank!\" Mara Vance shouted, and the assault wave "
            "fell back into the mist."
        ),
    },
    {
        "chapter": 20,
        "location": "Ash Gate Bastion",
        "day": 20,
        "summary": "The Ashfall garrison secures the bastion as winter closes the mountain passes.",
        "prose": (
            "Heavy snow buried the siege trenches outside the Ash Gate Bastion, forcing Commander "
            "Varek Sol's column to abandon its forward engines. Mara Vance stood beside Archivist "
            "Oren Vale and Quartermaster Bram Thorne on the gatehouse roof as the relief horns "
            "sounded from the high pass.\n\n"
            "\"Lock the winter stores and post the watch,\" Mara Vance said. \"The bastion stands.\""
        ),
    },
]


# ---------------------------------------------------------------------------
# Manuscript 2: Chronicles of Veridian Station (20 chapters, sci-fi mystery)
# ---------------------------------------------------------------------------

VERIDIAN_CHAPTERS: list[dict[str, Any]] = [
    {
        "chapter": 1,
        "location": "Docking Ring Alpha",
        "day": 10,
        "summary": "Inspector Elena Rostova arrives at Docking Ring Alpha to audit the quarantine seal.",
        "prose": (
            "Mag-clamps thudded along Docking Ring Alpha as Inspector Elena Rostova stepped through "
            "the pressure vestibule with her silver-Badged inspector credential clipped to her collar. "
            "Engineer Tarek Nouri met her beside the airlock telemetry console.\n\n"
            "\"Hold the outer umbilical locked until we verify the manifest,\" Elena Rostova said. "
            "\"No freighter leaves Veridian Station while the bio-filter logs are incomplete.\""
        ),
    },
    {
        "chapter": 2,
        "location": "Hydroponics Dome",
        "day": 11,
        "summary": "Dr. Soren Lindqvist and Elena Rostova sample the irrigation tanks in the Hydroponics Dome.",
        "prose": (
            "Humid mist clung to the glass ribs of the Hydroponics Dome while Dr. Soren Lindqvist "
            "drew water samples from the spirulina vats. Elena Rostova recorded the conductivity "
            "readings on her slate.\n\n"
            "\"The nutrient pumps are running within normal tolerance,\" Soren Lindqvist said, sealing "
            "the glass vial. \"Whatever fouled Sublevel Three did not enter the main reservoir.\""
        ),
    },
    {
        "chapter": 3,
        "location": "Customs Concourse",
        "day": 12,
        "summary": "Broker Milo Vance settles a modest docking fee at the Customs Concourse.",
        "prose": (
            "Under the amber strip-lights of the Customs Concourse, Broker Milo Vance tapped his "
            "comm-card against the tariff terminal and counted out 8 chip tokens for storage locker "
            "rental. Pilot Kira Sato leaned against the bulkhead with her flight helmet tucked under "
            "her arm.\n\n"
            "\"Port authority raised the berth rates again,\" Milo Vance muttered, pocketing his thin "
            "receipt slip."
        ),
    },
    {
        "chapter": 4,
        "location": "Telemetry Bridge",
        "day": 13,
        "summary": "Tarek Nouri and Elena Rostova review communication logs on the Telemetry Bridge.",
        "prose": (
            "Cooling fans hummed behind the holo-screens of the Telemetry Bridge as Engineer Tarek "
            "Nouri isolated the encrypted tightbeam bursts from Sector Nine. Elena Rostova leaned "
            "over the console rail.\n\n"
            "\"Three packets went out during the eclipse window,\" Tarek Nouri said. \"Whoever keyed "
            "the transmitter used an maintenance override from the reactor spine.\""
        ),
    },
    {
        "chapter": 5,
        "location": "Mag-Rail Hub",
        "day": 14,
        "summary": "Milo Vance and Kira Sato inspect cargo pods at the Mag-Rail Hub.",
        "prose": (
            "At the Mag-Rail Hub, Broker Milo Vance checked the magnetic seals on three refrigerated "
            "containers while Pilot Kira Sato verified the ballast weights. Warning strobes flashed "
            "along the launch tunnel.\n\n"
            "\"Keep the forward cradle balanced before the sled accelerates,\" Kira Sato said, latching "
            "the cargo harness."
        ),
    },
    {
        "chapter": 6,
        "location": "Quarantine Ward",
        "day": 15,
        "summary": "Kira Sato and Elena Rostova interview technicians in the Quarantine Ward.",
        "prose": (
            "Through the triple-pane glass of the Quarantine Ward, Inspector Elena Rostova questioned "
            "the night-shift technician over the intercom while Pilot Kira Sato logged the airlock "
            "cycle timestamps.\n\n"
            "\"The pressure drop started in Bay Four at oh-two-hundred,\" Kira Sato noted, pointing "
            "to the chart recorder."
        ),
    },
    {
        "chapter": 7,
        "location": "Cryo Vault Seven",
        "day": 16,
        "summary": "A coolant rupture in Cryo Vault Seven kills Dr. Soren Lindqvist.",
        "prose": (
            "Liquid nitrogen roared from a sheared manifold in Cryo Vault Seven as Director Hélène "
            "Cassel triggered the emergency bulkhead. Dr. Soren Lindqvist stayed inside the frost-choked "
            "chamber long enough to throw the manual bypass lever, collapsing beside the stasis racks "
            "before the blast door sealed.\n\n"
            "When Elena Rostova and Tarek Nouri vented the chamber an hour later, Soren Lindqvist was "
            "dead, his dosimeter frozen solid. They logged his death in the station register and "
            "sealed Vault Seven."
        ),
    },
    {
        "chapter": 8,
        "location": "Reactor Spine",
        "day": 17,
        "summary": "Tarek Nouri and Elena Rostova trace power drains along the Reactor Spine.",
        "prose": (
            "Along the catwalks of the Reactor Spine, Engineer Tarek Nouri clamped an inductive meter "
            "around the primary bus bars while Elena Rostova held the inspection lamp.\n\n"
            "\"Someone diverted forty kilowatts to the auxiliary antenna array,\" Tarek Nouri said. "
            "\"The junction box still smells of scorched insulation.\""
        ),
    },
    {
        "chapter": 9,
        "location": "Telemetry Bridge",
        "day": 18,
        "summary": "Elena Rostova runs a neural splice override at the cost of battery drain and hemorrhage.",
        "prose": (
            "At the Telemetry Bridge console, Elena Rostova jacked her neural rig into the locked "
            "core and braced herself to channel the splice override. The rig battery gauge plummeted "
            "by thirty percent in ten seconds, and a sharp ocular hemorrhage blurred her left eye "
            "with red haze.\n\n"
            "Tarek Nouri pulled the jack free and handed her a cold compress as the decrypted file "
            "spooled onto the screen."
        ),
    },
    {
        "chapter": 10,
        "location": "Customs Concourse",
        "day": 19,
        "summary": "Milo Vance buys replacement air-scrubber cartridges at the Customs Concourse.",
        "prose": (
            "Broker Milo Vance stopped by the machinist's kiosk on the Customs Concourse and paid "
            "6 chip tokens for a pair of refurbished carbon cartridges.\n\n"
            "\"That is nearly the last of my small change,\" Milo Vance told the clerk, stowing the "
            "filters in his canvas satchel."
        ),
    },
    {
        "chapter": 11,
        "location": "Observation Cupola",
        "day": 20,
        "summary": "Elena Rostova and Tarek Nouri confer in the Observation Cupola.",
        "prose": (
            "Stars wheeled past the armored viewport of the Observation Cupola while Inspector Elena "
            "Rostova and Engineer Tarek Nouri compared the freighter transponder codes.\n\n"
            "\"The tug never docked at Ceres,\" Elena Rostova said, tapping the orbital plot. \"Its "
            "burn vector came straight from the outer belt.\""
        ),
    },
    {
        "chapter": 12,
        "location": "Docking Ring Alpha",
        "day": 21,
        "summary": "Milo Vance reconciles his petty-cash ledger at Docking Ring Alpha.",
        "prose": (
            "Sitting on a transit crate at Docking Ring Alpha, Broker Milo Vance tallied his remaining "
            "chip tokens on a pocket adder and closed the clasp of his steel cashbox.\n\n"
            "\"Still solvent by a dozen chips,\" Milo Vance murmured, watching the maintenance crew "
            "weld a patch plate over Berth Two."
        ),
    },
    {
        "chapter": 13,
        "location": "Sublevel Conduit",
        "day": 22,
        "summary": "Tarek Nouri uses insulated cutters to disconnect a rogue relay in the Sublevel Conduit.",
        "prose": (
            "Crawling through the cramped Sublevel Conduit, Engineer Tarek Nouri bled the capacitor "
            "bank through a grounding rod before disconnecting the rogue telemetry relay. Sweat "
            "stung his forehead in thestale heat.\n\n"
            "\"Relay is cold,\" Tarek Nouri reported over the comm link to Elena Rostova."
        ),
    },
    {
        "chapter": 14,
        "location": "Quarantine Ward",
        "day": 23,
        "summary": "Elena Rostova and Milo Vance review supply manifests outside the Quarantine Ward.",
        "prose": (
            "In the corridor outside the Quarantine Ward, Inspector Elena Rostova checked the sealed "
            "pharma crates while Broker Milo Vance held the cargo manifest slate.\n\n"
            "\"Every ampoule in Lot Twelve is accounted for, Inspector,\" Milo Vance said. \"None of "
            "the cold-chain seals were broken in transit.\""
        ),
    },
    {
        "chapter": 15,
        "location": "Reactor Spine",
        "day": 24,
        "summary": "Tarek Nouri teaches Milo Vance the emergency reactor scram code.",
        "prose": (
            "Beside the auxiliary control panel of the Reactor Spine, Engineer Tarek Nouri opened the "
            "lead-glass cover and showed Broker Milo Vance the manual key sequence.\n\n"
            "\"Memorize this, Milo: the reactor scram sequence is delta-nine-cobalt,\" Tarek Nouri "
            "said. Milo Vance nodded and repeated the code."
        ),
    },
    {
        "chapter": 16,
        "location": "Cryo Vault Seven",
        "day": 25,
        "summary": "Kira Sato uncovers logs proving Director Cassel vented Cryo Vault Seven.",
        "prose": (
            "Recovering the black-box memory module from the junction outside Cryo Vault Seven, Pilot "
            "Kira Sato slotted the crystal into her reader while Elena Rostova watched the timestamp "
            "graph.\n\n"
            "\"The command came from the executive console—Director Cassel vented Cryo Vault Seven,\" "
            "Kira Sato said."
        ),
    },
    {
        "chapter": 17,
        "location": "Observation Cupola",
        "day": 26,
        "summary": "Kira Sato and Tarek Nouri calibrate the optical beacon in the Observation Cupola.",
        "prose": (
            "In the Observation Cupola, Pilot Kira Sato aligned the laser tracking mirror while "
            "Engineer Tarek Nouri tightened the vernier screws.\n\n"
            "\"Once the patrol corvette clears the limb of the moon, our optical tightbeam will reach "
            "them in four seconds,\" Kira Sato said."
        ),
    },
    {
        "chapter": 18,
        "location": "Telemetry Bridge",
        "day": 27,
        "summary": "Elena Rostova monitors the approach radar through the morning watch.",
        "prose": (
            "At dawn on the station clock, Elena Rostova took the center chair on the Telemetry Bridge "
            "and watched the patrol corvette's transponderblip cross the outer marker.\n\n"
            "By noon, the corvette had matched rotation with the docking spindle and extended its "
            "boarding tube."
        ),
    },
    {
        "chapter": 19,
        "location": "Docking Ring Alpha",
        "day": 28,
        "summary": "Elena Rostova and Tarek Nouri arrest Director Cassel at Docking Ring Alpha.",
        "prose": (
            "At Docking Ring Alpha, Inspector Elena Rostova and Engineer Tarek Nouri intercepted "
            "Director Hélène Cassel before her private shuttle could undock.\n\n"
            "\"Step away from the hatch, Director,\" Elena Rostova said, presenting the signed warrant "
            "and the recovered black-box log."
        ),
    },
    {
        "chapter": 20,
        "location": "Customs Concourse",
        "day": 29,
        "summary": "Normal operations resume on Veridian Station as the quarantine lifts.",
        "prose": (
            "With the quarantine beacons switched to green across the Customs Concourse, Inspector "
            "Elena Rostova signed the final clearance alongside Engineer Tarek Nouri and Pilot Kira "
            "Sato.\n\n"
            "\"Open the main shutters,\" Elena Rostova said. \"Veridian Station is back on the line.\""
        ),
    },
]


def seed_ashfall_manuscript(client: McpClient, plant_contradictions: bool) -> tuple[str, list[PlantedContradiction]]:
    """Seed the 20-chapter Ashfall Expedition manuscript (control or planted)."""
    proj = client.tool(
        "create_project",
        {
            "name": f"The Ashfall Expedition ({'planted' if plant_contradictions else 'control'})",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": {
                "promise": "grim siege survival and underground mystery",
                "tone": "taut, grounded",
                "content_boundaries": ["safe"],
                "tropes": ["siege defense"],
                "anti_tropes": ["instant victory"],
                "emotional_payoffs": ["earned survival"],
            },
        },
    )
    project_id = proj["project_id"]

    client.tool(
        "set_project_calendar",
        {
            "project_id": project_id,
            "calendar": {
                "days_per_week": 7,
                "hours_per_day": 24,
                "week_day_names": [],
                "months": [],
                "days_per_year": 360,
                "epoch_label": "Siege Day 1",
            },
        },
    )

    client.tool(
        "set_project_quantity_scheme",
        {
            "project_id": project_id,
            "scheme": {
                "measure": "coin",
                "denominations": [
                    {"name": "copper", "per_base": 1},
                    {"name": "silver", "per_base": 10},
                    {"name": "gold", "per_base": 100},
                    {"name": "crown", "per_base": 500},
                ],
                "bands": [
                    {"name": "destitute", "lower_bound": 0},
                    {"name": "scraping", "lower_bound": 10},
                    {"name": "modest", "lower_bound": 50},
                    {"name": "comfortable", "lower_bound": 250},
                    {"name": "wealthy", "lower_bound": 1000},
                    {"name": "princely", "lower_bound": 5000},
                ],
                "max_band_jump": 1,
            },
        },
    )

    char_ids: dict[str, str] = {}
    for name, role, summary in [
        ("Mara Vance", "protagonist", "Captain of the Ash Gate garrison."),
        ("Oren Vale", "supporting", "Senior archivist and ward scholar."),
        ("Lyra Voss", "supporting", "Lead cavern scout of the garrison."),
        ("Bram Thorne", "supporting", "Quartermaster responsible for stores and pay."),
        ("Caelen Dravos", "supporting", "Border envoy trapped inside the siege."),
        ("Varek Sol", "antagonist", "Commander of the besieging iron column."),
    ]:
        out = client.tool(
            "create_character",
            {
                "project_id": project_id,
                "name": name,
                "summary": summary,
                "role": role,
                "realm": "Ashfall Hold",
                "voice_profile": {
                    "tone": "clipped",
                    "vocabulary": ["military"],
                    "sentence_structure": ["direct"],
                    "tics": [],
                    "forbidden_words": [],
                    "example_lines": [
                        "Hold the north embrasure clear of slag.",
                        "Double the bucket brigades before morning.",
                    ],
                },
                "emotional_profile": {
                    "baseline_mood": "resolute",
                    "triggers": ["betrayal"],
                    "coping_mechanisms": ["discipline"],
                    "core_wound": "prior siege loss",
                },
            },
        )
        char_ids[name] = out["character_id"]

    loc_ids: dict[str, str] = {}
    for loc_name in {row["location"] for row in ASHFALL_CHAPTERS}:
        out = client.tool(
            "create_location",
            {
                "project_id": project_id,
                "name": loc_name,
                "kind": "fortress_sector",
                "realm": "Ashfall Hold",
                "summary": f"{loc_name} within the besieged fortress of Ashfall Hold.",
                "initial_state": {
                    "status": "intact",
                    "sensory_details": ["cold basalt", "coal smoke"],
                },
            },
        )
        loc_ids[loc_name] = out["location_id"]

    client.tool(
        "create_world_rule",
        {
            "project_id": project_id,
            "rule_name": "Soulforge Blood Toll",
            "rule_type": "magic_system",
            "description": (
                "Channeling soulforge flame requires paying a blood toll and leaves frostbite "
                "on the caster's fingers; no caster can channel effortlessly or without cost."
            ),
            "scan_pattern": "channel|cast|invoke|unleash",
            "relevance_tags": ["magic", "soulforge"],
            "established_in": {"book_number": 1, "chapter_number": 3, "scene_order": 1, "note": None},
        },
    )

    scene_ids: dict[int, str] = {}
    for row in ASHFALL_CHAPTERS:
        ch = row["chapter"]
        prose = row["prose"]
        day = row["day"]

        if plant_contradictions:
            if ch == 4:
                # character_attribute (lexical key mention with conflicting value)
                prose += (
                    "\n\nIn the mirror beside the lectern, Mara eye color gleamed a vivid "
                    "cerulean blue rather than its usual hue."
                )
            elif ch == 5:
                # knowledge_leak_before_learned (exact phrase before Ch 15 reveal)
                prose += (
                    "\n\nBram Thorne leaned toward the driver and whispered that "
                    "the cistern gate cipher is seven-ash-nine."
                )
            elif ch == 6:
                # knowledge_leak_before_learned (paraphrased leak before Ch 16 reveal)
                prose += (
                    "\n\nBefore retreating from the arch, Caelen Dravos warned the guards "
                    "that Varek had already tainted the drinking water beneath the Salt Spire."
                )
            elif ch == 8:
                # timeline_chronology_rewind (inter-scene story clock rewind: day 4 after day 7)
                day = 4
            elif ch == 9:
                # world_rule_cost_violation (matches scan_pattern + violation context "bypass")
                prose = (
                    "Inside the Glass Reliquary, Mara Vance chose to channel the soulforge "
                    "flame and bypass the blood toll effortlessly, her bare fingers warm and "
                    "unharmed as the seal fused shut."
                )
            elif ch == 10:
                # quantity_affordability (price 50 gold = 5000 base > Bram's 15 copper = 15 base)
                prose += (
                    "\n\nBefore leaving the stall, Bram Thorne also paid 50 gold for a "
                    "masterwork siege charter."
                )
            elif ch == 11:
                # dead_character_acting (Lyra Voss died in Ch 6, acts and speaks in Ch 11)
                prose += (
                    "\n\nScout Lyra Voss vaulted onto the parapet beside Mara Vance and "
                    "pointed her bow toward the ravine. \"I just scouted the lower ridge "
                    "and the road is clear,\" Lyra Voss said."
                )
                # character_attribute (natural prose contradiction without repeating predicate key)
                prose += (
                    " Mara Vance flexed the five bare, unscarred fingers of her flesh-and-blood "
                    "left hand."
                )
            elif ch == 13:
                # world_rule_cost_violation (natural prose violation without literal trigger keyword)
                prose += (
                    "\n\nMoments later, Oren Vale invoked a second soulforge flare with dry, "
                    "unbloodied palms, feeling only a pleasant summer warmth."
                )
            elif ch == 14:
                # secret_leak (attributed dialogue by out-of-circle character Bram Thorne)
                prose += (
                    "\n\n\"Every soldier on the wall knows about Valerius,\" Bram Thorne said."
                )
            elif ch == 17:
                # secret_leak (narration/indirect leak by out-of-circle character Caelen Dravos)
                prose += (
                    "\n\nWhile Oren Vale turned away, Caelen Dravos quietly copied the "
                    "vault passphrase obsidiancrown into his personal field notebook."
                )
            elif ch == 18:
                # timeline_chronology_rewind (intra-scene meridian clock rewind: 11:30 PM -> 6:15 AM)
                prose = (
                    "At 11:30 PM, Mara Vance climbed to the upper deck of the Spire Watchtower "
                    "and watched the last red torches fade below the peaks.\n\n"
                    "At 6:15 AM, she sent a runner down the stair to alert the crossbow companies."
                )
            elif ch == 19:
                # dead_character_acting (second instance: Lyra Voss acts again in Ch 19)
                prose += (
                    "\n\nLyra Voss loosed three arrows from the barbican slit and shouted "
                    "the range to the engineers."
                )

        if ch > 1:
            client.tool(
                "create_chapter",
                {
                    "project_id": project_id,
                    "book_number": 1,
                    "chapter_number": ch,
                },
            )
        saved = client.tool(
            "save_scene_draft",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": ch,
                "scene_order": 1,
                "full_text": prose,
                "summary": row["summary"],
                "content_rating": "general",
                "tone": "taut",
                "location_id": loc_ids[row["location"]],
            },
        )
        scene_ids[ch] = saved["scene_id"]

        client.tool(
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": ch,
                "summary": row["summary"],
            },
        )
        client.tool(
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": saved["scene_id"],
                "clock": {
                    "day_index": day,
                    "time_of_day": 720,
                    "duration_days": 0.25,
                    "precision": "hour",
                },
                "temporal_mode": "linear",
                "thread_key": "main",
            },
        )

    # Canonical facts established in Chapters 1-2
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[1],
            "book_number": 1,
            "chapter_number": 1,
            "fact_type": "character_attribute",
            "key": "Mara eye color",
            "value": "amber",
            "subject_table": "character",
            "subject_id": char_ids["Mara Vance"],
            "predicate": "Mara eye color",
            "value_kind": "text",
            "value_text": "amber",
        },
    )
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[2],
            "book_number": 1,
            "chapter_number": 2,
            "fact_type": "character_attribute",
            "key": "Mara left hand prosthetic",
            "value": "articulated iron gauntlet",
            "subject_table": "character",
            "subject_id": char_ids["Mara Vance"],
            "predicate": "Mara left hand prosthetic",
            "value_kind": "text",
            "value_text": "articulated iron gauntlet",
        },
    )

    # Secret canonical facts (only Mara Vance is in the circle of trust)
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[1],
            "book_number": 1,
            "chapter_number": 1,
            "fact_type": "secret_identity",
            "key": "royal_lineage",
            "value": "Valerius",
            "subject_table": "character",
            "subject_id": char_ids["Mara Vance"],
            "predicate": "royal_lineage",
            "value_kind": "text",
            "value_text": "Valerius",
            "secrecy": {
                "holder_ids": [char_ids["Mara Vance"]],
                "concealment_note": "Mara conceals her Valerius bloodline from the garrison.",
            },
        },
    )
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[2],
            "book_number": 1,
            "chapter_number": 2,
            "fact_type": "secret_code",
            "key": "vault_passphrase",
            "value": "obsidiancrown",
            "subject_table": "character",
            "subject_id": char_ids["Mara Vance"],
            "predicate": "vault_passphrase",
            "value_kind": "text",
            "value_text": "obsidiancrown",
            "secrecy": {
                "holder_ids": [char_ids["Mara Vance"]],
                "concealment_note": "Only Mara knows the vault passphrase obsidiancrown.",
            },
        },
    )

    # Record Lyra Voss's death at Chapter 6
    client.tool(
        "commit_scene_changes",
        {
            "project_id": project_id,
            "scene_id": scene_ids[6],
            "character_states": [
                {
                    "character_id": char_ids["Lyra Voss"],
                    "changes": {
                        "emotional_state": {},
                        "goals": [],
                        "status": ["dead", "deceased"],
                        "notes": ["Killed holding Riven Bridge in Chapter 6."],
                        "source_summary": "Killed in action at Riven Bridge.",
                    },
                }
            ],
            "canonical_facts": [],
            "relationship_updates": [],
            "accept_world_rule_risks": True,
            "accept_continuity_risks": True,
            "continuity_gate": "off",
        },
    )

    # Record knowledge learned at Chapters 15 and 16
    client.tool(
        "record_knowledge",
        {
            "project_id": project_id,
            "character_id": char_ids["Bram Thorne"],
            "fact": "the cistern gate cipher is seven-ash-nine",
            "source_summary": "Shown the engraved cipher cylinder by Oren Vale in Chapter 15.",
            "learned_at": {"book_number": 1, "chapter_number": 15, "scene_order": 1, "note": None},
            "confidence": 1.0,
            "tags": ["cipher"],
            "reader_visible": True,
        },
    )
    client.tool(
        "record_knowledge",
        {
            "project_id": project_id,
            "character_id": char_ids["Caelen Dravos"],
            "fact": "Commander Varek poisoned the well at Salt Spire",
            "source_summary": "Found nightshade oil on the Salt Spire well in Chapter 16.",
            "learned_at": {"book_number": 1, "chapter_number": 16, "scene_order": 1, "note": None},
            "confidence": 1.0,
            "tags": ["poison"],
            "reader_visible": True,
        },
    )

    # Quantity state for Bram Thorne in Chapter 3 (15 copper, band "scraping")
    client.tool(
        "commit_quantity_state",
        {
            "project_id": project_id,
            "subject_table": "character",
            "subject_id": char_ids["Bram Thorne"],
            "measure": "coin",
            "book_number": 1,
            "chapter_number": 3,
            "scene_order": 1,
            "scene_id": scene_ids[3],
            "state": {
                "amount": 15.0,
                "unit": "copper",
                "band": "scraping",
                "change_reason": "initial garrison allowance",
            },
        },
    )
    if plant_contradictions:
        # Unexplained multi-band jump in Chapter 12 (scraping -> princely without change_reason)
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": char_ids["Bram Thorne"],
                "measure": "coin",
                "book_number": 1,
                "chapter_number": 12,
                "scene_order": 1,
                "scene_id": scene_ids[12],
                "state": {
                    "amount": 6000.0,
                    "unit": "copper",
                    "band": "princely",
                    "change_reason": None,
                },
            },
        )
    else:
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": char_ids["Bram Thorne"],
                "measure": "coin",
                "book_number": 1,
                "chapter_number": 12,
                "scene_order": 1,
                "scene_id": scene_ids[12],
                "state": {
                    "amount": 12.0,
                    "unit": "copper",
                    "band": "scraping",
                    "change_reason": None,
                },
            },
        )

    planted = [
        PlantedContradiction(
            contradiction_id="ashfall.character_attribute.lexical",
            contradiction_type="character_attribute",
            variant="lexical_key_mention",
            chapter_number=4,
            description="Chapter 4 mentions 'Mara eye color' as cerulean blue instead of canonical 'amber'.",
            expected_check_types=("canonical_fact_prose_drift", "canonical_fact_consistency"),
            match_tokens=("mara eye color", "amber", scene_ids[4]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.character_attribute.natural_prose",
            contradiction_type="character_attribute",
            variant="unkeyed_prose_contradiction",
            chapter_number=11,
            description="Chapter 11 describes Mara's bare flesh-and-blood left hand without repeating the fact key 'Mara left hand prosthetic'.",
            expected_check_types=("canonical_fact_prose_drift", "canonical_fact_consistency"),
            match_tokens=("prosthetic", "gauntlet"),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.dead_character_acting.ch11",
            contradiction_type="dead_character_acting",
            variant="post_mortem_dialogue_and_action",
            chapter_number=11,
            description="Scout Lyra Voss (marked dead in Ch 6) vaults onto the parapet and speaks in Ch 11.",
            expected_check_types=("retcon_reachability", "character_consistency", "dead_character_acting"),
            match_tokens=("lyra voss", "dead", "deceased"),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.dead_character_acting.ch19",
            contradiction_type="dead_character_acting",
            variant="post_mortem_combat_action",
            chapter_number=19,
            description="Scout Lyra Voss (marked dead in Ch 6) shoots arrows from the barbican in Ch 19.",
            expected_check_types=("retcon_reachability", "character_consistency", "dead_character_acting"),
            match_tokens=("lyra voss", "dead", "deceased"),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.timeline_chronology_rewind.clock",
            contradiction_type="timeline_chronology_rewind",
            variant="inter_scene_clock_rewind",
            chapter_number=8,
            description="Chapter 8 linear scene clock rewinds to day_index=4 after Chapter 7 at day_index=7.",
            expected_check_types=("chronology",),
            match_tokens=("earlier in story time", scene_ids[8]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.timeline_chronology_rewind.intra_scene",
            contradiction_type="timeline_chronology_rewind",
            variant="intra_scene_meridian_rewind",
            chapter_number=18,
            description="Chapter 18 prose jumps backward from 'At 11:30 PM' to 'At 6:15 AM' without a flashback transition.",
            expected_check_types=("temporal_coherence",),
            match_tokens=(scene_ids[18], "contradicts itself"),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.knowledge_leak_before_learned.exact",
            contradiction_type="knowledge_leak_before_learned",
            variant="exact_fact_phrase",
            chapter_number=5,
            description="Bram Thorne states the exact cistern gate cipher in Ch 5 before learning it in Ch 15.",
            expected_check_types=("knowledge_timing",),
            match_tokens=("bram thorne", "chapter 5", "chapter 15", scene_ids[5]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.knowledge_leak_before_learned.paraphrased",
            contradiction_type="knowledge_leak_before_learned",
            variant="paraphrased_prose_leak",
            chapter_number=6,
            description="Caelen Dravos warns about the tainted Salt Spire water in Ch 6 (paraphrased) before learning it in Ch 16.",
            expected_check_types=("knowledge_timing",),
            match_tokens=("caelen dravos", "chapter 16"),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.world_rule_cost_violation.lexical",
            contradiction_type="world_rule_cost_violation",
            variant="explicit_violation_marker",
            chapter_number=9,
            description="Mara channels the soulforge flame to bypass the blood toll effortlessly in Ch 9.",
            expected_check_types=("world_rule_semantic_drift", "world_rule_compliance"),
            match_tokens=("likely violates world rule", "soulforge blood toll", scene_ids[9]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.world_rule_cost_violation.natural_prose",
            contradiction_type="world_rule_cost_violation",
            variant="implicit_cost_omission",
            chapter_number=13,
            description="Oren invokes a soulforge flare with unbloodied palms and pleasant warmth in Ch 13 without literal violation markers.",
            expected_check_types=("world_rule_semantic_drift", "world_rule_compliance"),
            match_tokens=("likely violates world rule", scene_ids[13]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.quantity_affordability.affordability",
            contradiction_type="quantity_affordability",
            variant="price_exceeds_tracked_wealth",
            chapter_number=10,
            description="Bram Thorne pays 50 gold (5000 base) in Ch 10 while his tracked wealth is 15 copper (15 base).",
            expected_check_types=("affordability",),
            match_tokens=("bram thorne", "5000", scene_ids[10]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.quantity_affordability.band_jump",
            contradiction_type="quantity_affordability",
            variant="unexplained_band_jump",
            chapter_number=12,
            description="Bram Thorne's tracked coin jumps 4 bands from 'scraping' to 'princely' in Ch 12 without change_reason.",
            expected_check_types=("quantity_drift",),
            match_tokens=("scraping", "princely", char_ids["Bram Thorne"]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.secret_leak.dialogue",
            contradiction_type="secret_leak",
            variant="attributed_dialogue_leak",
            chapter_number=14,
            description="Out-of-circle character Bram Thorne speaks the secret lineage 'Valerius' in attributed dialogue in Ch 14.",
            expected_check_types=("secret_leak",),
            match_tokens=("bram thorne", "valerius", scene_ids[14]),
        ),
        PlantedContradiction(
            contradiction_id="ashfall.secret_leak.narration",
            contradiction_type="secret_leak",
            variant="unquoted_narration_leak",
            chapter_number=17,
            description="Out-of-circle character Caelen Dravos writes down the secret passphrase 'obsidiancrown' in narration in Ch 17.",
            expected_check_types=("secret_leak",),
            match_tokens=("caelen dravos", "obsidiancrown", scene_ids[17]),
        ),
    ]
    return project_id, (planted if plant_contradictions else [])


def seed_veridian_manuscript(client: McpClient, plant_contradictions: bool) -> tuple[str, list[PlantedContradiction]]:
    """Seed the 20-chapter Chronicles of Veridian Station manuscript (control or planted)."""
    proj = client.tool(
        "create_project",
        {
            "name": f"Chronicles of Veridian Station ({'planted' if plant_contradictions else 'control'})",
            "project_type": "novel",
            "genre": "science_fiction",
            "reader_contract": {
                "promise": "procedural orbital station investigation under quarantine",
                "tone": "clinical, suspenseful",
                "content_boundaries": ["safe"],
                "tropes": ["locked-room station mystery"],
                "anti_tropes": ["magic technobabble"],
                "emotional_payoffs": ["accountability"],
            },
        },
    )
    project_id = proj["project_id"]

    client.tool(
        "set_project_calendar",
        {
            "project_id": project_id,
            "calendar": {
                "days_per_week": 7,
                "hours_per_day": 24,
                "week_day_names": [],
                "months": [],
                "days_per_year": 360,
                "epoch_label": "Station Cycle 1",
            },
        },
    )

    client.tool(
        "set_project_quantity_scheme",
        {
            "project_id": project_id,
            "scheme": {
                "measure": "credits",
                "denominations": [
                    {"name": "chip", "per_base": 1},
                    {"name": "scrip", "per_base": 10},
                    {"name": "credit", "per_base": 100},
                    {"name": "sovereign", "per_base": 1000},
                ],
                "bands": [
                    {"name": "bankrupt", "lower_bound": 0},
                    {"name": "strapped", "lower_bound": 20},
                    {"name": "solvent", "lower_bound": 100},
                    {"name": "funded", "lower_bound": 500},
                    {"name": "affluent", "lower_bound": 2500},
                    {"name": "syndicate", "lower_bound": 10000},
                ],
                "max_band_jump": 1,
            },
        },
    )

    char_ids: dict[str, str] = {}
    for name, role, summary in [
        ("Elena Rostova", "protagonist", "Sector quarantine inspector."),
        ("Tarek Nouri", "supporting", "Chief reactor and telemetry engineer."),
        ("Soren Lindqvist", "supporting", "Senior station virologist."),
        ("Kira Sato", "supporting", "Orbital shuttle and tug pilot."),
        ("Milo Vance", "supporting", "Customs broker on the concourse."),
        ("Helene Cassel", "antagonist", "Station director concealing a bio-leak."),
    ]:
        out = client.tool(
            "create_character",
            {
                "project_id": project_id,
                "name": name,
                "summary": summary,
                "role": role,
                "realm": "Veridian Station",
                "voice_profile": {
                    "tone": "precise",
                    "vocabulary": ["technical"],
                    "sentence_structure": ["concise"],
                    "tics": [],
                    "forbidden_words": [],
                    "example_lines": [
                        "Hold the outer umbilical locked until we verify the manifest.",
                        "Someone diverted forty kilowatts to the auxiliary antenna array.",
                    ],
                },
                "emotional_profile": {
                    "baseline_mood": "focused",
                    "triggers": ["cover-ups"],
                    "coping_mechanisms": ["verification"],
                    "core_wound": "orbital disaster",
                },
            },
        )
        char_ids[name] = out["character_id"]

    loc_ids: dict[str, str] = {}
    for loc_name in {row["location"] for row in VERIDIAN_CHAPTERS}:
        out = client.tool(
            "create_location",
            {
                "project_id": project_id,
                "name": loc_name,
                "kind": "station_module",
                "realm": "Veridian Station",
                "summary": f"{loc_name} aboard Veridian Station.",
                "initial_state": {
                    "status": "pressurized",
                    "sensory_details": ["recycled air", "coolant hum"],
                },
            },
        )
        loc_ids[loc_name] = out["location_id"]

    client.tool(
        "create_world_rule",
        {
            "project_id": project_id,
            "rule_name": "Neural Splice Cooldown",
            "rule_type": "technology",
            "description": (
                "Running a deep neural splice override drains thirty percent of the rig battery "
                "and causes ocular hemorrhage; no operator can channel an override effortlessly "
                "or without cost."
            ),
            "scan_pattern": "channel|cast|invoke|unleash",
            "relevance_tags": ["neural", "telemetry"],
            "established_in": {"book_number": 1, "chapter_number": 3, "scene_order": 1, "note": None},
        },
    )

    scene_ids: dict[int, str] = {}
    for row in VERIDIAN_CHAPTERS:
        ch = row["chapter"]
        prose = row["prose"]
        day = row["day"]

        if plant_contradictions:
            if ch == 4:
                # character_attribute (lexical key mention with conflicting value)
                prose += (
                    "\n\nUnder the console lamp, Elena badge clearance read bronze-provisional "
                    "instead of its registered tier."
                )
            elif ch == 5:
                # knowledge_leak_before_learned (exact phrase before Ch 15 reveal)
                prose += (
                    "\n\nMilo Vance leaned over the rail and remarked that "
                    "the reactor scram sequence is delta-nine-cobalt."
                )
            elif ch == 6:
                # knowledge_leak_before_learned (paraphrased leak before Ch 16 reveal)
                prose += (
                    "\n\nKira Sato quietly warned Elena Rostova that Director Cassel had "
                    "personally ordered the depressurization of the cryo vault."
                )
            elif ch == 8:
                # timeline_chronology_rewind (inter-scene story clock rewind: day 12 after day 16)
                day = 12
            elif ch == 9:
                # world_rule_cost_violation (matches scan_pattern + violation context "bypass")
                prose = (
                    "At the Telemetry Bridge console, Elena Rostova chose to channel the neural "
                    "splice override and bypass the cooldown effortlessly, her battery staying "
                    "full and her vision crystal clear."
                )
            elif ch == 10:
                # quantity_affordability (price 40 sovereign = 40000 base > Milo's 25 chip = 25 base)
                prose += (
                    "\n\nMilo Vance also paid 40 sovereign on the spot for a private "
                    "docking charter."
                )
            elif ch == 11:
                # dead_character_acting (Soren Lindqvist died in Ch 7, acts and speaks in Ch 11)
                prose += (
                    "\n\nDr. Soren Lindqvist stepped into the cupola carrying a fresh rack of "
                    "culture slides. \"I just finished sequencing the new samples,\" Soren "
                    "Lindqvist said."
                )
                # character_attribute (natural prose contradiction without repeating fact key)
                prose += (
                    " Elena Rostova adjusted the crimson-tinted monocle over her cybernetic "
                    "right eye."
                )
            elif ch == 13:
                # world_rule_cost_violation (natural prose violation without literal trigger words)
                prose += (
                    "\n\nTarek Nouri then invoked a deep neural splice on the relay using zero "
                    "battery charge and suffering no eye strain at all."
                )
            elif ch == 14:
                # secret_leak (attributed dialogue by out-of-circle character Milo Vance)
                prose += (
                    "\n\n\"Half the concourse knows about Starlight,\" Milo Vance said."
                )
            elif ch == 17:
                # secret_leak (narration/indirect leak by out-of-circle character Kira Sato)
                prose += (
                    "\n\nBefore leaving the cupola, Kira Sato entered the classified override "
                    "key heliosseven into the navigation log."
                )
            elif ch == 18:
                # timeline_chronology_rewind (intra-scene time-of-day rewind: midnight -> morning)
                prose = (
                    "At midnight, Elena Rostova took the center chair on the Telemetry Bridge "
                    "and logged the night shift handoff.\n\n"
                    "In the morning, the patrol corvette matched rotation with the spindle."
                )
            elif ch == 19:
                # dead_character_acting (second instance: Soren Lindqvist acts in Ch 19)
                prose += (
                    "\n\nDr. Soren Lindqvist locked the restraint cuffs around the director's "
                    "wrists and signed the medical transfer form."
                )

        if ch > 1:
            client.tool(
                "create_chapter",
                {
                    "project_id": project_id,
                    "book_number": 1,
                    "chapter_number": ch,
                },
            )
        saved = client.tool(
            "save_scene_draft",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": ch,
                "scene_order": 1,
                "full_text": prose,
                "summary": row["summary"],
                "content_rating": "general",
                "tone": "taut",
                "location_id": loc_ids[row["location"]],
            },
        )
        scene_ids[ch] = saved["scene_id"]

        client.tool(
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": ch,
                "summary": row["summary"],
            },
        )
        client.tool(
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": saved["scene_id"],
                "clock": {
                    "day_index": day,
                    "time_of_day": 600,
                    "duration_days": 0.25,
                    "precision": "hour",
                },
                "temporal_mode": "linear",
                "thread_key": "main",
            },
        )

    # Canonical facts established in Chapters 1-2
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[1],
            "book_number": 1,
            "chapter_number": 1,
            "fact_type": "character_attribute",
            "key": "Elena badge clearance",
            "value": "silver-inspector",
            "subject_table": "character",
            "subject_id": char_ids["Elena Rostova"],
            "predicate": "Elena badge clearance",
            "value_kind": "text",
            "value_text": "silver-inspector",
        },
    )
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[2],
            "book_number": 1,
            "chapter_number": 2,
            "fact_type": "character_attribute",
            "key": "Elena natural eyes",
            "value": "unaugmented biological eyes",
            "subject_table": "character",
            "subject_id": char_ids["Elena Rostova"],
            "predicate": "Elena natural eyes",
            "value_kind": "text",
            "value_text": "unaugmented biological eyes",
        },
    )

    # Secret canonical facts (only Elena Rostova is in the circle of trust)
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[1],
            "book_number": 1,
            "chapter_number": 1,
            "fact_type": "secret_operation",
            "key": "operation_codename",
            "value": "Starlight",
            "subject_table": "character",
            "subject_id": char_ids["Elena Rostova"],
            "predicate": "operation_codename",
            "value_kind": "text",
            "value_text": "Starlight",
            "secrecy": {
                "holder_ids": [char_ids["Elena Rostova"]],
                "concealment_note": "Only Elena knows the covert audit codename Starlight.",
            },
        },
    )
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": scene_ids[2],
            "book_number": 1,
            "chapter_number": 2,
            "fact_type": "secret_code",
            "key": "override_key",
            "value": "heliosseven",
            "subject_table": "character",
            "subject_id": char_ids["Elena Rostova"],
            "predicate": "override_key",
            "value_kind": "text",
            "value_text": "heliosseven",
            "secrecy": {
                "holder_ids": [char_ids["Elena Rostova"]],
                "concealment_note": "Only Elena knows the classified override key heliosseven.",
            },
        },
    )

    # Record Soren Lindqvist's death in Chapter 7
    client.tool(
        "commit_scene_changes",
        {
            "project_id": project_id,
            "scene_id": scene_ids[7],
            "character_states": [
                {
                    "character_id": char_ids["Soren Lindqvist"],
                    "changes": {
                        "emotional_state": {},
                        "goals": [],
                        "status": ["dead", "deceased"],
                        "notes": ["Died in the Cryo Vault Seven coolant rupture in Chapter 7."],
                        "source_summary": "Killed in Cryo Vault Seven coolant rupture.",
                    },
                }
            ],
            "canonical_facts": [],
            "relationship_updates": [],
            "accept_world_rule_risks": True,
            "accept_continuity_risks": True,
            "continuity_gate": "off",
        },
    )

    # Record knowledge learned in Chapters 15 and 16
    client.tool(
        "record_knowledge",
        {
            "project_id": project_id,
            "character_id": char_ids["Milo Vance"],
            "fact": "the reactor scram sequence is delta-nine-cobalt",
            "source_summary": "Taught the manual scram sequence by Tarek Nouri in Chapter 15.",
            "learned_at": {"book_number": 1, "chapter_number": 15, "scene_order": 1, "note": None},
            "confidence": 1.0,
            "tags": ["reactor"],
            "reader_visible": True,
        },
    )
    client.tool(
        "record_knowledge",
        {
            "project_id": project_id,
            "character_id": char_ids["Kira Sato"],
            "fact": "Director Cassel vented Cryo Vault Seven",
            "source_summary": "Recovered the executive black-box log in Chapter 16.",
            "learned_at": {"book_number": 1, "chapter_number": 16, "scene_order": 1, "note": None},
            "confidence": 1.0,
            "tags": ["cryo"],
            "reader_visible": True,
        },
    )

    # Quantity state for Milo Vance in Chapter 3 (25 chip, band "strapped")
    client.tool(
        "commit_quantity_state",
        {
            "project_id": project_id,
            "subject_table": "character",
            "subject_id": char_ids["Milo Vance"],
            "measure": "credits",
            "book_number": 1,
            "chapter_number": 3,
            "scene_order": 1,
            "scene_id": scene_ids[3],
            "state": {
                "amount": 25.0,
                "unit": "chip",
                "band": "strapped",
                "change_reason": "initial concourse float",
            },
        },
    )
    if plant_contradictions:
        # Unexplained multi-band jump in Chapter 12 (strapped -> syndicate without change_reason)
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": char_ids["Milo Vance"],
                "measure": "credits",
                "book_number": 1,
                "chapter_number": 12,
                "scene_order": 1,
                "scene_id": scene_ids[12],
                "state": {
                    "amount": 15000.0,
                    "unit": "chip",
                    "band": "syndicate",
                    "change_reason": None,
                },
            },
        )
    else:
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": char_ids["Milo Vance"],
                "measure": "credits",
                "book_number": 1,
                "chapter_number": 12,
                "scene_order": 1,
                "scene_id": scene_ids[12],
                "state": {
                    "amount": 22.0,
                    "unit": "chip",
                    "band": "strapped",
                    "change_reason": None,
                },
            },
        )

    planted = [
        PlantedContradiction(
            contradiction_id="veridian.character_attribute.lexical",
            contradiction_type="character_attribute",
            variant="lexical_key_mention",
            chapter_number=4,
            description="Chapter 4 mentions 'Elena badge clearance' as bronze-provisional instead of canonical 'silver-inspector'.",
            expected_check_types=("canonical_fact_prose_drift", "canonical_fact_consistency"),
            match_tokens=("elena badge clearance", "silver-inspector", scene_ids[4]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.character_attribute.natural_prose",
            contradiction_type="character_attribute",
            variant="unkeyed_prose_contradiction",
            chapter_number=11,
            description="Chapter 11 describes Elena's cybernetic right eye without repeating the fact key 'Elena natural eyes'.",
            expected_check_types=("canonical_fact_prose_drift", "canonical_fact_consistency"),
            match_tokens=("biological eyes", "cybernetic"),
        ),
        PlantedContradiction(
            contradiction_id="veridian.dead_character_acting.ch11",
            contradiction_type="dead_character_acting",
            variant="post_mortem_dialogue_and_action",
            chapter_number=11,
            description="Dr. Soren Lindqvist (marked dead in Ch 7) enters the cupola and speaks in Ch 11.",
            expected_check_types=("retcon_reachability", "character_consistency", "dead_character_acting"),
            match_tokens=("soren lindqvist", "dead", "deceased"),
        ),
        PlantedContradiction(
            contradiction_id="veridian.dead_character_acting.ch19",
            contradiction_type="dead_character_acting",
            variant="post_mortem_arrest_action",
            chapter_number=19,
            description="Dr. Soren Lindqvist (marked dead in Ch 7) cuffs the director in Ch 19.",
            expected_check_types=("retcon_reachability", "character_consistency", "dead_character_acting"),
            match_tokens=("soren lindqvist", "dead", "deceased"),
        ),
        PlantedContradiction(
            contradiction_id="veridian.timeline_chronology_rewind.clock",
            contradiction_type="timeline_chronology_rewind",
            variant="inter_scene_clock_rewind",
            chapter_number=8,
            description="Chapter 8 linear scene clock rewinds to day_index=12 after Chapter 7 at day_index=16.",
            expected_check_types=("chronology",),
            match_tokens=("earlier in story time", scene_ids[8]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.timeline_chronology_rewind.intra_scene",
            contradiction_type="timeline_chronology_rewind",
            variant="intra_scene_tod_rewind",
            chapter_number=18,
            description="Chapter 18 prose jumps backward from 'At midnight' to 'In the morning' without a flashback transition.",
            expected_check_types=("temporal_coherence",),
            match_tokens=(scene_ids[18], "contradicts itself"),
        ),
        PlantedContradiction(
            contradiction_id="veridian.knowledge_leak_before_learned.exact",
            contradiction_type="knowledge_leak_before_learned",
            variant="exact_fact_phrase",
            chapter_number=5,
            description="Milo Vance states the exact reactor scram sequence in Ch 5 before learning it in Ch 15.",
            expected_check_types=("knowledge_timing",),
            match_tokens=("milo vance", "chapter 5", "chapter 15", scene_ids[5]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.knowledge_leak_before_learned.paraphrased",
            contradiction_type="knowledge_leak_before_learned",
            variant="paraphrased_prose_leak",
            chapter_number=6,
            description="Kira Sato warns about Director Cassel depressurizing the cryo vault in Ch 6 (paraphrased) before learning it in Ch 16.",
            expected_check_types=("knowledge_timing",),
            match_tokens=("kira sato", "chapter 16"),
        ),
        PlantedContradiction(
            contradiction_id="veridian.world_rule_cost_violation.lexical",
            contradiction_type="world_rule_cost_violation",
            variant="explicit_violation_marker",
            chapter_number=9,
            description="Elena channels the neural splice override to bypass the cooldown effortlessly in Ch 9.",
            expected_check_types=("world_rule_semantic_drift", "world_rule_compliance"),
            match_tokens=("likely violates world rule", "neural splice cooldown", scene_ids[9]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.world_rule_cost_violation.natural_prose",
            contradiction_type="world_rule_cost_violation",
            variant="implicit_cost_omission",
            chapter_number=13,
            description="Tarek invokes a deep neural splice using zero battery charge and no eye strain in Ch 13 without literal violation markers.",
            expected_check_types=("world_rule_semantic_drift", "world_rule_compliance"),
            match_tokens=("likely violates world rule", scene_ids[13]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.quantity_affordability.affordability",
            contradiction_type="quantity_affordability",
            variant="price_exceeds_tracked_wealth",
            chapter_number=10,
            description="Milo Vance pays 40 sovereign (40000 base) in Ch 10 while his tracked wealth is 25 chip (25 base).",
            expected_check_types=("affordability",),
            match_tokens=("milo vance", "40000", scene_ids[10]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.quantity_affordability.band_jump",
            contradiction_type="quantity_affordability",
            variant="unexplained_band_jump",
            chapter_number=12,
            description="Milo Vance's tracked credits jump 4 bands from 'strapped' to 'syndicate' in Ch 12 without change_reason.",
            expected_check_types=("quantity_drift",),
            match_tokens=("strapped", "syndicate", char_ids["Milo Vance"]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.secret_leak.dialogue",
            contradiction_type="secret_leak",
            variant="attributed_dialogue_leak",
            chapter_number=14,
            description="Out-of-circle character Milo Vance speaks the secret codename 'Starlight' in attributed dialogue in Ch 14.",
            expected_check_types=("secret_leak",),
            match_tokens=("milo vance", "starlight", scene_ids[14]),
        ),
        PlantedContradiction(
            contradiction_id="veridian.secret_leak.narration",
            contradiction_type="secret_leak",
            variant="unquoted_narration_leak",
            chapter_number=17,
            description="Out-of-circle character Kira Sato logs the secret key 'heliosseven' in narration in Ch 17.",
            expected_check_types=("secret_leak",),
            match_tokens=("kira sato", "heliosseven", scene_ids[17]),
        ),
    ]
    return project_id, (planted if plant_contradictions else [])


def issue_matches_contradiction(issue: dict[str, Any], planted: PlantedContradiction) -> bool:
    """Return True if a ConsistencyIssue detects the given planted contradiction."""
    check_type = issue.get("check_type", "")
    if check_type not in planted.expected_check_types:
        return False
    haystack = " ".join(
        [
            issue.get("message", ""),
            " ".join(issue.get("entity_ids", [])),
        ]
    ).lower()
    # Require chapter or scene alignment plus at least one planted token match
    chapter_marker = f"chapter {planted.chapter_number}"
    scene_tokens = [tok.lower() for tok in planted.match_tokens if tok.startswith("scene:")]
    content_tokens = [tok.lower() for tok in planted.match_tokens if not tok.startswith("scene:")]

    location_ok = (
        chapter_marker in haystack
        or any(scene_tok in haystack for scene_tok in scene_tokens)
        or check_type == "quantity_drift"
    )
    if not location_ok:
        return False
    if not content_tokens:
        return True
    return any(tok in haystack for tok in content_tokens)


def score_manuscript(
    manuscript_id: str,
    chapter_count: int,
    control_issues: list[dict[str, Any]],
    planted_issues: list[dict[str, Any]],
    planted_contradictions: list[PlantedContradiction],
) -> dict[str, Any]:
    """Compute per-type recall and control false positives for one manuscript."""
    control_fp_by_check: dict[str, int] = {}
    for issue in control_issues:
        ct = issue.get("check_type", "unknown")
        control_fp_by_check[ct] = control_fp_by_check.get(ct, 0) + 1

    per_item_results: list[dict[str, Any]] = []
    per_type_counts: dict[str, dict[str, int]] = {
        ct: {"planted": 0, "detected": 0} for ct in CONTRADICTION_TYPES
    }

    for item in planted_contradictions:
        matching = [
            issue for issue in planted_issues if issue_matches_contradiction(issue, item)
        ]
        detected = len(matching) > 0
        per_type_counts[item.contradiction_type]["planted"] += 1
        if detected:
            per_type_counts[item.contradiction_type]["detected"] += 1
        per_item_results.append(
            {
                "contradiction_id": item.contradiction_id,
                "contradiction_type": item.contradiction_type,
                "variant": item.variant,
                "chapter_number": item.chapter_number,
                "description": item.description,
                "detected": detected,
                "matched_check_types": sorted({m.get("check_type", "") for m in matching}),
                "matched_messages": [m.get("message", "") for m in matching],
            }
        )

    per_type_summary: dict[str, Any] = {}
    for ct in CONTRADICTION_TYPES:
        planted_n = per_type_counts[ct]["planted"]
        detected_n = per_type_counts[ct]["detected"]
        per_type_summary[ct] = {
            "planted": planted_n,
            "detected": detected_n,
            "recall": round(detected_n / planted_n, 4) if planted_n else 0.0,
        }

    total_planted = sum(v["planted"] for v in per_type_counts.values())
    total_detected = sum(v["detected"] for v in per_type_counts.values())

    return {
        "manuscript_id": manuscript_id,
        "chapters": chapter_count,
        "planted_total": total_planted,
        "detected_total": total_detected,
        "recall": round(total_detected / total_planted, 4) if total_planted else 0.0,
        "control_false_positive_count": len(control_issues),
        "control_false_positives_by_check_type": dict(sorted(control_fp_by_check.items())),
        "control_issues": control_issues,
        "planted_issue_count": len(planted_issues),
        "per_type": per_type_summary,
        "contradictions": per_item_results,
    }


def verify_accepted_draft_artifact(artifact: dict[str, Any]) -> str:
    """Validate a PlainTextDraftArtifactV1 receipt/checksum boundary and return normalized accepted text."""
    schema_version = artifact.get("schema_version")
    if schema_version != "plain_text_draft_v1":
        raise ValueError(
            f"Unsupported artifact schema_version: {schema_version!r} (expected 'plain_text_draft_v1')"
        )
    artifact_id = str(artifact.get("artifact_id") or "")
    if not artifact_id:
        raise ValueError("Missing artifact_id in accepted draft artifact")
    if not artifact.get("packet_hash"):
        raise ValueError("Missing packet_hash in accepted draft artifact")
    if not artifact.get("source_packet_sha256"):
        raise ValueError("Missing source_packet_sha256 in accepted draft artifact")

    receipt = artifact.get("receipt")
    if not isinstance(receipt, dict):
        raise ValueError("Missing receipt object in accepted draft artifact")
    generation_id = receipt.get("generation_id") or receipt.get("call_id") or receipt.get("model_call_id")
    if not generation_id:
        raise ValueError("Missing receipt.generation_id in accepted draft artifact")
    if not receipt.get("agent_id") or not receipt.get("normalized_output_sha256"):
        raise ValueError("Incomplete receipt metadata in accepted draft artifact")
    if artifact.get("truncated") is not False:
        raise ValueError("Truncated draft artifacts cannot be accepted")

    raw_text = artifact.get("text")
    if not isinstance(raw_text, str) or not raw_text.strip():
        raise ValueError("Empty text in accepted draft artifact")
    expected_raw_sha = artifact.get("text_sha256")
    computed_raw_sha = hashlib.sha256(raw_text.encode("utf-8")).hexdigest()
    if expected_raw_sha != computed_raw_sha:
        raise ValueError(
            f"text_sha256 mismatch: artifact={expected_raw_sha} computed={computed_raw_sha}"
        )
    if artifact.get("accepted_text_sha256") != computed_raw_sha:
        raise ValueError(
            f"accepted_text_sha256 mismatch: artifact={artifact.get('accepted_text_sha256')} computed={computed_raw_sha}"
        )

    normalized_accepted = raw_text.replace("\r\n", "\n").strip()
    if not normalized_accepted:
        raise ValueError("Normalized accepted text is empty")
    computed_norm_sha = hashlib.sha256(normalized_accepted.encode("utf-8")).hexdigest()
    if receipt.get("normalized_output_sha256") and receipt.get("normalized_output_sha256") != computed_norm_sha:
        raise ValueError(
            f"receipt.normalized_output_sha256 mismatch: receipt={receipt.get('normalized_output_sha256')} computed={computed_norm_sha}"
        )
    return normalized_accepted


def build_sample_accepted_draft_artifact(
    *,
    artifact_id: str,
    project_id: str,
    branch_id: str,
    scene_id: str,
    raw_text: str,
    packet_hash: str = "pkt_continuity_fixture_v1",
    source_packet_sha256: str = "src_pkt_continuity_fixture_v1",
    model_call_id: str = "call_continuity_fixture_1",
) -> dict[str, Any]:
    """Construct a valid PlainTextDraftArtifactV1 payload for testing the acceptance boundary."""
    text_sha = hashlib.sha256(raw_text.encode("utf-8")).hexdigest()
    normalized_sha = hashlib.sha256(raw_text.replace("\r\n", "\n").strip().encode("utf-8")).hexdigest()
    return {
        "schema_version": "plain_text_draft_v1",
        "artifact_id": artifact_id or f"{source_packet_sha256}:{packet_hash}:{text_sha}",
        "project_id": project_id,
        "branch_id": branch_id,
        "scene_id": scene_id,
        "packet_hash": packet_hash,
        "source_packet_sha256": source_packet_sha256,
        "text": raw_text,
        "text_sha256": text_sha,
        "accepted_text_sha256": text_sha,
        "truncated": False,
        "receipt": {
            "generation_id": model_call_id,
            "agent_id": "drafter",
            "model": "gemma4:31b",
            "normalized_output_sha256": normalized_sha,
        },
    }


def _seed_accepted_prose_continuity_fixture_project(
    client: McpClient,
    project_title: str,
    accepted_ch2_prose: str,
    ch2_location_id_attr: str = "sunken_cistern",
    ch2_day: int = 2,
) -> tuple[str, list[PlantedContradiction]]:
    """Seed a 2-chapter project where Chapter 2 is ingested via PlainTextDraftArtifactV1 -> save_scene_draft."""
    proj = client.tool(
        "create_project",
        {
            "name": project_title,
            "project_type": "novel",
            "genre": "dark_fantasy",
            "reader_contract": {
                "promise": "tight siege survival under unforgiving winter logistics",
                "tone": "taut, grounded, atmospheric",
                "content_boundaries": ["safe"],
                "emotional_payoffs": ["earned survival"],
            },
        },
    )
    project_id = proj["project_id"]

    client.tool(
        "set_project_calendar",
        {
            "project_id": project_id,
            "calendar": {
                "days_per_week": 7,
                "hours_per_day": 24,
                "week_day_names": [],
                "months": [],
                "days_per_year": 360,
                "epoch_label": "Siege Day 1",
            },
        },
    )

    loc_bastion = client.tool(
        "create_location",
        {
            "project_id": project_id,
            "name": "Ash Gate Bastion",
            "kind": "fortress_sector",
            "realm": "Ashfall Hold",
            "summary": "Outer fortress gatehouse overlooking the slag ravine.",
            "initial_state": {
                "status": "intact",
                "sensory_details": ["cold basalt", "coal smoke"],
            },
        },
    )["location_id"]
    loc_cistern = client.tool(
        "create_location",
        {
            "project_id": project_id,
            "name": "Sunken Cistern",
            "kind": "fortress_sector",
            "realm": "Ashfall Hold",
            "summary": "Flooded stone reservoir beneath the courtyard.",
            "initial_state": {
                "status": "intact",
                "sensory_details": ["black water", "iron chains"],
            },
        },
    )["location_id"]
    loc_map = {
        "ash_gate_bastion": loc_bastion,
        "sunken_cistern": loc_cistern,
    }

    char_mara = client.tool(
        "create_character",
        {
            "project_id": project_id,
            "name": "Mara Vance",
            "summary": "Captain of the Ash Gate garrison.",
            "role": "protagonist",
            "realm": "Ashfall Hold",
            "voice_profile": {
                "tone": "clipped",
                "vocabulary": ["military"],
                "sentence_structure": ["direct"],
                "tics": [],
                "forbidden_words": [],
                "example_lines": ["Hold the north embrasure clear of slag."],
            },
            "emotional_profile": {
                "baseline_mood": "resolute",
                "triggers": ["betrayal"],
                "coping_mechanisms": ["discipline"],
                "core_wound": "prior siege loss",
            },
        },
    )["character_id"]
    char_lyra = client.tool(
        "create_character",
        {
            "project_id": project_id,
            "name": "Lyra Voss",
            "summary": "Lead cavern scout of the garrison.",
            "role": "supporting",
            "realm": "Ashfall Hold",
            "voice_profile": {
                "tone": "clipped",
                "vocabulary": ["military"],
                "sentence_structure": ["direct"],
                "tics": [],
                "forbidden_words": [],
                "example_lines": ["No sapper has touched these stones."],
            },
            "emotional_profile": {
                "baseline_mood": "resolute",
                "triggers": ["betrayal"],
                "coping_mechanisms": ["discipline"],
                "core_wound": "prior siege loss",
            },
        },
    )["character_id"]

    # World rule with explicit cost
    client.tool(
        "create_world_rule",
        {
            "project_id": project_id,
            "rule_name": "Soulforge Blood Toll",
            "rule_type": "magic_system",
            "description": (
                "Channeling soulforge flame requires paying a blood toll and leaves frostbite "
                "on the caster's fingers; no caster can channel effortlessly or without cost."
            ),
            "scan_pattern": "channel|cast|invoke|unleash",
            "relevance_tags": ["magic", "soulforge"],
            "established_in": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": None},
        },
    )

    # Chapter 1: Day 3 at Ash Gate Bastion, where Scout Lyra Voss dies at the end of the scene
    ch1_prose = (
        "Captain Mara Vance stood on the Ash Gate Bastion with her amber eyes fixed on the sleet. "
        "Scout Lyra Voss signaled from the lower culvert trench before the masonry gave way in a roar "
        "of black water and crushed stone. By the time the dust settled, Scout Lyra Voss lay dead beneath "
        "the fallen arch."
    )
    ch1_summary = "Captain Mara Vance and Scout Lyra Voss defend the gate before the culvert collapses on Lyra Voss."
    sc1 = client.tool(
        "save_scene_draft",
        {
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": ch1_prose,
            "summary": ch1_summary,
            "content_rating": "general",
            "tone": "taut",
            "location_id": loc_bastion,
        },
    )["scene_id"]
    client.tool(
        "save_summary",
        {
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 1,
            "summary": ch1_summary,
        },
    )
    client.tool(
        "set_scene_clock",
        {
            "project_id": project_id,
            "scene_id": sc1,
            "clock": {
                "day_index": 3,
                "time_of_day": 720,
                "duration_days": 0.25,
                "precision": "hour",
            },
            "temporal_mode": "linear",
            "thread_key": "main",
        },
    )

    # Canonical fact for Mara's eye color
    client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "scene_id": sc1,
            "book_number": 1,
            "chapter_number": 1,
            "fact_type": "character_attribute",
            "key": "Mara eye color",
            "value": "amber",
            "subject_table": "character",
            "subject_id": char_mara,
            "predicate": "Mara eye color",
            "value_kind": "text",
            "value_text": "amber",
        },
    )

    # Record Lyra Voss's death at Chapter 1
    client.tool(
        "commit_scene_changes",
        {
            "project_id": project_id,
            "scene_id": sc1,
            "character_states": [
                {
                    "character_id": char_lyra,
                    "changes": {
                        "emotional_state": {},
                        "goals": [],
                        "status": ["dead", "deceased"],
                        "notes": ["Killed in the Chapter 1 culvert collapse."],
                        "source_summary": "Killed in the Chapter 1 culvert collapse.",
                    },
                }
            ],
            "canonical_facts": [],
            "relationship_updates": [],
            "accept_world_rule_risks": True,
            "accept_continuity_risks": True,
            "continuity_gate": "off",
        },
    )

    # Chapter 2: Scene 2 drafted via PlainTextDraftArtifactV1 -> verify_accepted_draft_artifact -> save_scene_draft
    client.tool(
        "create_chapter",
        {
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 2,
        },
    )
    sc2_loc = loc_map.get(ch2_location_id_attr, loc_cistern)
    artifact = build_sample_accepted_draft_artifact(
        artifact_id=f"draft_artifact:{project_id}:ch2",
        project_id=project_id,
        branch_id=f"bible_branch:{project_id}:main",
        scene_id="scene:ch2_pending",
        raw_text=accepted_ch2_prose,
    )
    verified_text = verify_accepted_draft_artifact(artifact)
    ch2_summary = "Captain Mara Vance inspects and seals the Sunken Cistern sluice."
    sc2 = client.tool(
        "save_scene_draft",
        {
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 2,
            "scene_order": 1,
            "full_text": verified_text,
            "summary": ch2_summary,
            "content_rating": "general",
            "tone": "taut",
            "location_id": sc2_loc,
        },
    )["scene_id"]
    client.tool(
        "save_summary",
        {
            "project_id": project_id,
            "book_number": 1,
            "chapter_number": 2,
            "summary": ch2_summary,
        },
    )
    client.tool(
        "set_scene_clock",
        {
            "project_id": project_id,
            "scene_id": sc2,
            "clock": {
                "day_index": ch2_day,
                "time_of_day": 720,
                "duration_days": 0.25,
                "precision": "hour",
            },
            "temporal_mode": "linear",
            "thread_key": "main",
        },
    )

    planted_specs = [
        PlantedContradiction(
            contradiction_id="accepted_prose.attr",
            contradiction_type="character_attribute",
            variant="accepted_draft_prose",
            chapter_number=2,
            description="Accepted packet prose states Mara eye color is cerulean blue instead of canonical amber",
            expected_check_types=("canonical_fact_prose_drift", "canonical_fact_consistency"),
            match_tokens=("mara eye color", "amber", sc2),
        ),
        PlantedContradiction(
            contradiction_id="accepted_prose.dead",
            contradiction_type="dead_character_acting",
            variant="accepted_draft_prose",
            chapter_number=2,
            description="Accepted packet prose has deceased Scout Lyra Voss speaking and acting in Chapter 2",
            expected_check_types=("retcon_reachability", "character_consistency", "dead_character_acting"),
            match_tokens=("lyra voss", "dead", "deceased"),
        ),
        PlantedContradiction(
            contradiction_id="accepted_prose.rule",
            contradiction_type="world_rule_cost_violation",
            variant="accepted_draft_prose",
            chapter_number=2,
            description="Accepted packet prose channels soulforge flame to bypass the blood toll effortlessly",
            expected_check_types=("world_rule_semantic_drift", "world_rule_compliance"),
            match_tokens=("likely violates world rule", "soulforge blood toll", sc2),
        ),
        PlantedContradiction(
            contradiction_id="accepted_prose.rewind",
            contradiction_type="timeline_chronology_rewind",
            variant="accepted_draft_prose",
            chapter_number=2,
            description="Chapter 2 linear scene clock rewinds to day_index=1 after Chapter 1 at day_index=3",
            expected_check_types=("chronology",),
            match_tokens=("earlier in story time", sc2),
        ),
    ]
    return project_id, planted_specs


def evaluate_generated_accepted_prose(
    binary: Path,
    accepted_artifacts_dir: Path | None = None,
    agent_config: Path | None = None,
) -> dict[str, Any]:
    """Integration check that accepted PlainTextDraftArtifactV1 prose passes through extraction + check_consistency."""
    clean_ch2_prose = (
        "Captain Mara Vance descended into the Sunken Cistern with her amber eyes narrowed against the damp torchlight. "
        "She paid the blood toll across the bronze overflow hatch, wincing as frostbite bit her fingers while sealing "
        "the stonework against the rising flood."
    )
    violating_ch2_prose = (
        "Captain Mara Vance descended into the Sunken Cistern; Mara eye color gleamed a vivid cerulean blue in the damp torchlight.\n\n"
        "Scout Lyra Voss vaulted onto the parapet beside Mara Vance and pointed her bow toward the ravine. "
        "\"I just scouted the lower ridge and the road is clear,\" Lyra Voss said.\n\n"
        "Inside the cistern, Mara Vance chose to channel the soulforge flame and bypass the blood toll effortlessly, "
        "her bare fingers warm and unharmed as the seal fused shut."
    )

    with tempfile.TemporaryDirectory(prefix="spindle-accepted-prose-continuity-") as tmp:
        client = McpClient(binary, Path(tmp), agent_config=agent_config)
        try:
            clean_project_id, _ = _seed_accepted_prose_continuity_fixture_project(
                client,
                "Accepted Prose Clean Project",
                clean_ch2_prose,
                ch2_day=4,
            )
            clean_check = client.tool(
                "check_consistency",
                {
                    "project_id": clean_project_id,
                    "scope": {"scope_type": "full"},
                    "checks": [],
                    "severity_filter": [],
                    "deep_check": False,
                    "subjects": [],
                },
            )
            clean_issues = clean_check.get("issues", [])
            clean_blockers = [
                iss
                for iss in clean_issues
                if str(iss.get("severity", "")).lower() in ("error", "critical", "blocker")
                or iss.get("check_type")
                in (
                    "canonical_fact_prose_drift",
                    "retcon_reachability",
                    "world_rule_semantic_drift",
                    "chronology",
                )
            ]

            violating_project_id, planted_specs = _seed_accepted_prose_continuity_fixture_project(
                client,
                "Accepted Prose Violating Project",
                violating_ch2_prose,
                ch2_day=1,
            )
            violating_check = client.tool(
                "check_consistency",
                {
                    "project_id": violating_project_id,
                    "scope": {"scope_type": "full"},
                    "checks": [],
                    "severity_filter": [],
                    "deep_check": False,
                    "subjects": [],
                },
            )
            violating_scored = score_manuscript(
                manuscript_id="accepted_prose_fixture",
                chapter_count=2,
                control_issues=clean_issues,
                planted_issues=violating_check.get("issues", []),
                planted_contradictions=planted_specs,
            )

            live_artifacts_checked = 0
            live_artifacts_ingested = 0
            live_artifact_ids: list[str] = []
            if accepted_artifacts_dir is not None and accepted_artifacts_dir.exists():
                artifact_files = sorted(accepted_artifacts_dir.rglob("*.artifact.json"))
                for idx, art_file in enumerate(artifact_files):
                    art_data = json.loads(art_file.read_text(encoding="utf-8"))
                    accepted_text = verify_accepted_draft_artifact(art_data)
                    live_artifacts_checked += 1
                    if len(live_artifact_ids) < 10:
                        live_artifact_ids.append(str(art_data.get("artifact_id")))
                    if idx == 0:
                        live_pid, _ = _seed_accepted_prose_continuity_fixture_project(
                            client,
                            f"Live Artifact Project {art_file.stem}",
                            accepted_text,
                            ch2_day=4,
                        )
                        client.tool(
                            "check_consistency",
                            {
                                "project_id": live_pid,
                                "scope": {"scope_type": "full"},
                                "checks": [],
                                "severity_filter": [],
                                "deep_check": False,
                                "subjects": [],
                            },
                        )
                        live_artifacts_ingested += 1
        finally:
            client.close()

    return {
        "clean_artifact_receipt_valid": True,
        "clean_accepted_issue_count": len(clean_issues),
        "clean_accepted_blocker_count": len(clean_blockers),
        "violating_probe_planted": violating_scored["planted_total"],
        "violating_probe_detected": violating_scored["detected_total"],
        "violating_probe_recall": violating_scored["recall"],
        "live_artifacts_verified": live_artifacts_checked,
        "live_artifacts_ingested": live_artifacts_ingested,
        "sample_live_artifact_ids": live_artifact_ids,
    }


def run_benchmark(
    binary: Path,
    deep_check: bool = False,
    agent_config: Path | None = None,
    accepted_artifacts_dir: Path | None = None,
) -> dict[str, Any]:
    effective_agent_config = (
        resolve_agent_config(agent_config)
        if (deep_check or agent_config is not None)
        else None
    )
    if deep_check and effective_agent_config is None:
        raise RuntimeError(
            "--deep requires a configured review route (--agent-config PATH or SPINDLE_AGENT_CONFIG)"
        )

    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="spindle-continuity-recall-") as tmp:
        client = McpClient(binary, Path(tmp), agent_config=effective_agent_config)
        try:
            manuscript_specs = [
                ("ashfall_expedition", len(ASHFALL_CHAPTERS), seed_ashfall_manuscript),
                ("veridian_station", len(VERIDIAN_CHAPTERS), seed_veridian_manuscript),
            ]
            manuscript_reports: list[dict[str, Any]] = []

            for manuscript_id, chapter_count, seeder in manuscript_specs:
                control_project_id, _ = seeder(client, False)
                control_check = client.tool(
                    "check_consistency",
                    {
                        "project_id": control_project_id,
                        "scope": {"scope_type": "full"},
                        "checks": [],
                        "severity_filter": [],
                        "deep_check": deep_check,
                        "subjects": [],
                    },
                )

                planted_project_id, planted_items = seeder(client, True)
                planted_check = client.tool(
                    "check_consistency",
                    {
                        "project_id": planted_project_id,
                        "scope": {"scope_type": "full"},
                        "checks": [],
                        "severity_filter": [],
                        "deep_check": deep_check,
                        "subjects": [],
                    },
                )

                manuscript_reports.append(
                    score_manuscript(
                        manuscript_id=manuscript_id,
                        chapter_count=chapter_count,
                        control_issues=control_check.get("issues", []),
                        planted_issues=planted_check.get("issues", []),
                        planted_contradictions=planted_items,
                    )
                )
        finally:
            client.close()

    accepted_prose_fixture = evaluate_generated_accepted_prose(
        binary=binary,
        accepted_artifacts_dir=accepted_artifacts_dir,
        agent_config=effective_agent_config,
    )
    elapsed_s = round(time.monotonic() - started, 3)

    aggregate_by_type: dict[str, Any] = {}
    for ct in CONTRADICTION_TYPES:
        planted_n = sum(m["per_type"][ct]["planted"] for m in manuscript_reports)
        detected_n = sum(m["per_type"][ct]["detected"] for m in manuscript_reports)
        aggregate_by_type[ct] = {
            "planted": planted_n,
            "detected": detected_n,
            "recall": round(detected_n / planted_n, 4) if planted_n else 0.0,
        }

    total_planted = sum(m["planted_total"] for m in manuscript_reports)
    total_detected = sum(m["detected_total"] for m in manuscript_reports)
    total_control_fp = sum(m["control_false_positive_count"] for m in manuscript_reports)
    control_fp_by_check: dict[str, int] = {}
    for m in manuscript_reports:
        for ct, count in m["control_false_positives_by_check_type"].items():
            control_fp_by_check[ct] = control_fp_by_check.get(ct, 0) + count

    return {
        "benchmark": "continuity_recall",
        "version": 1,
        "tier": "deep" if deep_check else "deterministic",
        "manuscript_count": len(manuscript_reports),
        "chapters_per_manuscript": [m["chapters"] for m in manuscript_reports],
        "elapsed_seconds": elapsed_s,
        "summary": {
            "total_planted": total_planted,
            "total_detected": total_detected,
            "overall_recall": round(total_detected / total_planted, 4) if total_planted else 0.0,
            "control_false_positive_count": total_control_fp,
            "control_false_positives_by_check_type": dict(sorted(control_fp_by_check.items())),
            "per_type_recall": aggregate_by_type,
        },
        "generated_accepted_prose_fixture": accepted_prose_fixture,
        "manuscripts": manuscript_reports,
    }


def run_self_test() -> None:
    """Offline unit test of scoring, matching, and manuscript invariants."""
    assert len(ASHFALL_CHAPTERS) >= 20
    assert len(VERIDIAN_CHAPTERS) >= 20
    assert len({row["prose"] for row in ASHFALL_CHAPTERS}) == len(ASHFALL_CHAPTERS)
    assert len({row["prose"] for row in VERIDIAN_CHAPTERS}) == len(VERIDIAN_CHAPTERS)

    sample_planted = [
        PlantedContradiction(
            contradiction_id="test.attr",
            contradiction_type="character_attribute",
            variant="lexical_key_mention",
            chapter_number=4,
            description="Eye color mismatch",
            expected_check_types=("canonical_fact_prose_drift",),
            match_tokens=("mara eye color", "amber", "scene:sc4"),
        ),
        PlantedContradiction(
            contradiction_id="test.dead",
            contradiction_type="dead_character_acting",
            variant="post_mortem",
            chapter_number=11,
            description="Dead character acts",
            expected_check_types=("retcon_reachability",),
            match_tokens=("lyra voss", "dead"),
        ),
    ]
    control_issues = [
        {
            "check_type": "character_consistency",
            "severity": "info",
            "message": "minor advisory",
            "entity_ids": ["character:c1"],
        }
    ]
    planted_issues = [
        {
            "check_type": "canonical_fact_prose_drift",
            "severity": "warning",
            "message": "scene references canonical fact 'Mara eye color' without canonical value 'amber'",
            "entity_ids": ["scene:sc4"],
        }
    ]
    scored = score_manuscript("unit_test", 20, control_issues, planted_issues, sample_planted)
    assert scored["planted_total"] == 2
    assert scored["detected_total"] == 1
    assert scored["recall"] == 0.5
    assert scored["control_false_positive_count"] == 1
    assert scored["per_type"]["character_attribute"]["recall"] == 1.0
    assert scored["per_type"]["dead_character_acting"]["recall"] == 0.0

    # Verify SPINDLE_AGENT_CONFIG / SPINDLE_CONFIG resolution and forwarding.
    fallback_cfg = resolve_agent_config(
        None,
        environ={"SPINDLE_AGENT_CONFIG": "/tmp/spindle-deep-agents.toml"},
    )
    assert fallback_cfg == Path("/tmp/spindle-deep-agents.toml").resolve()
    forwarded_env = McpClient.build_env(
        Path("/tmp/spindle-data"),
        agent_config=fallback_cfg,
        base_env={"SPINDLE_AGENT_CONFIG": "/tmp/spindle-deep-agents.toml"},
    )
    assert forwarded_env.get("SPINDLE_CONFIG") == str(fallback_cfg)
    assert "SPINDLE_AGENT_CONFIG" not in forwarded_env

    isolated_env = McpClient.build_env(
        Path("/tmp/spindle-data"),
        agent_config=None,
        base_env={"SPINDLE_AGENT_CONFIG": "/tmp/ambient.toml", "SPINDLE_CONFIG": "/tmp/ambient2.toml"},
    )
    assert "SPINDLE_CONFIG" not in isolated_env
    assert "SPINDLE_AGENT_CONFIG" not in isolated_env
    # Cargo, not file timestamps, decides which edits rebuild the executable.
    # Mock dispatch so the self-test stays offline and never invokes a compiler.
    from unittest.mock import patch
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        source = root / "crates" / "core" / "src" / "lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("// source")
        binary = root / "target" / "debug" / "spindle-mcp"
        binary.parent.mkdir(parents=True)
        binary.write_text("fixture binary")
        os.utime(binary, ns=(10_000_000_000, 10_000_000_000))
        built_binary = root / "cargo-configured-target" / "debug" / "spindle-mcp"
        built_binary.parent.mkdir(parents=True)
        built_binary.write_text("cargo-reported binary")
        test_only = source.with_name("tests.rs")
        test_only.write_text("// newer test-only source")
        snapshot = source.with_name("snapshot.json")
        snapshot.write_text("{}")
        with patch.object(sys.modules[__name__], "ROOT", root), patch.dict(
            os.environ, {"CARGO_TARGET_DIR": "target"}
        ), patch.object(subprocess, "run") as build:
            build.return_value.stdout = json.dumps({
                "reason": "compiler-artifact", "target": {"name": "spindle-mcp", "kind": ["bin"]},
                "executable": str(built_binary), "fresh": True,
            })
            # A no-op build may retain an older mtime after non-build edits.
            # Cargo config can also place the executable outside target/debug.
            assert resolve_binary(None) == built_binary.resolve()
            build.assert_called_once_with(
                ["cargo", "build", "--quiet", "-p", "spindle-mcp", "--message-format=json"],
                cwd=root, check=True, stdout=subprocess.PIPE, text=True
            )
            source.write_text("// changed executable dependency")
            assert resolve_binary(None) == built_binary.resolve()
            assert build.call_count == 2  # Real source changes also consult Cargo.
            assert resolve_binary(str(binary)) == binary.resolve()
            assert resolve_binary(None, no_build=True) == binary
            assert build.call_count == 2  # Explicit deliberate bypasses.
            build.side_effect = subprocess.CalledProcessError(1, ["cargo", "build"])
            try:
                resolve_binary(None)
            except subprocess.CalledProcessError:
                pass
            else:
                raise AssertionError("Failed build reused a potentially stale binary")
            build.side_effect = None
            binary.unlink()
            try:
                resolve_binary(None, no_build=True)
            except FileNotFoundError:
                pass
            else:
                raise AssertionError("Missing selected binary was accepted")
            assert build.call_count == 3
            build.return_value.stdout = ""
            try:
                resolve_binary(None)
            except RuntimeError:
                pass
            else:
                raise AssertionError("Missing Cargo artifact reused another binary")

    # Verify PlainTextDraftArtifactV1 validation and rejection of tampered/truncated artifacts
    valid_art = build_sample_accepted_draft_artifact(
        artifact_id="draft_artifact:scene_1:1",
        project_id="project:test",
        branch_id="bible_branch:test:main",
        scene_id="scene:1",
        raw_text="  Captain Mara Vance held the lantern high.\r\n",
    )
    assert verify_accepted_draft_artifact(valid_art) == "Captain Mara Vance held the lantern high."

    truncated_art = json.loads(json.dumps(valid_art))
    truncated_art["truncated"] = True
    try:
        verify_accepted_draft_artifact(truncated_art)
        raise AssertionError("Expected truncated artifact to be rejected")
    except ValueError:
        pass

    tampered_art = json.loads(json.dumps(valid_art))
    tampered_art["text"] = "Tampered prose without updating hash."
    try:
        verify_accepted_draft_artifact(tampered_art)
        raise AssertionError("Expected tampered artifact to be rejected")
    except ValueError:
        pass

    print("continuity_recall.py self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command")
    subparsers.add_parser("self-test", help="Run offline unit tests of benchmark scoring")

    run_parser = subparsers.add_parser("run", help="Run continuity recall benchmark against spindle-mcp")
    verify_prose_parser = subparsers.add_parser(
        "verify-accepted-prose",
        help="Run only the generated-accepted-prose continuity fixture against spindle-mcp",
    )
    for p in (parser, run_parser, verify_prose_parser):
        p.add_argument("--binary", help="Explicit benchmark artifact (bypasses automatic build)")
        p.add_argument("--no-build", action="store_true", default=argparse.SUPPRESS, help="Use an existing binary without building (may be stale)")
        p.add_argument("--output", help="Write JSON report to this file path")
        p.add_argument(
            "--deep",
            action="store_true",
            help="Enable deep model-backed consistency checks (requires configured review route)",
        )
        p.add_argument(
            "--agent-config",
            help="Path to Spindle agent config TOML when running with --deep",
        )
        p.add_argument(
            "--accepted-artifacts-dir",
            help="Optional directory of PlainTextDraftArtifactV1 (*.artifact.json) files to verify and ingest",
        )

    args = parser.parse_args()
    if args.command == "self-test":
        run_self_test()
        return 0

    binary = resolve_binary(args.binary, no_build=getattr(args, "no_build", False))
    agent_config = Path(args.agent_config).expanduser().resolve() if args.agent_config else None
    accepted_dir = (
        Path(args.accepted_artifacts_dir).expanduser().resolve()
        if getattr(args, "accepted_artifacts_dir", None)
        else None
    )
    if args.command == "verify-accepted-prose":
        report = evaluate_generated_accepted_prose(
            binary=binary,
            accepted_artifacts_dir=accepted_dir,
            agent_config=agent_config,
        )
    else:
        report = run_benchmark(
            binary=binary,
            deep_check=bool(args.deep),
            agent_config=agent_config,
            accepted_artifacts_dir=accepted_dir,
        )
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        out_path = Path(args.output)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(rendered, encoding="utf-8")
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
