#!/usr/bin/env python3
"""Blind held-out-4 continuity corpus.

Two independently authored 16-chapter manuscripts with clean and planted twins,
28 contradictions per manuscript (four in each public category).  The corpus is
deliberately written before the semantic lane exists; it records canon through
the same public MCP tools an author-agent uses and is scored by the shared
``continuity_recall`` scorer.

This module owns the corpus text, its expectations, the clean-twin audit and the
``held-out-4`` runner.  Nothing here knows how a detector is implemented.
"""

from __future__ import annotations

import hashlib
import json
import textwrap
import tempfile
from pathlib import Path
from typing import Any


def _p(text: str) -> str:
    return textwrap.dedent(text).strip()


def _plant(
    plant_id: str,
    contradiction_type: str,
    variant: str,
    chapter: int,
    description: str,
    expected_check_types: tuple[str, ...],
    match_tokens: tuple[str, ...],
    *,
    kind: str = "prose",
    anchor: str | None = None,
    text: str | None = None,
    clock_override: dict[str, Any] | None = None,
    quantity_state: dict[str, Any] | None = None,
    fact_key: str | None = None,
    learned_at_chapter: int | None = None,
    death_chapter: int | None = None,
    clean_forbidden: tuple[str, ...] = (),
) -> dict[str, Any]:
    return {
        "id": plant_id,
        "type": contradiction_type,
        "variant": variant,
        "chapter": chapter,
        "description": description,
        "expected_check_types": expected_check_types,
        "match_tokens": match_tokens,
        "kind": kind,
        "anchor": anchor,
        "text": text,
        "clock_override": clock_override,
        "quantity_state": quantity_state,
        "fact_key": fact_key,
        "learned_at_chapter": learned_at_chapter,
        "death_chapter": death_chapter,
        "clean_forbidden": clean_forbidden,
    }


# ---------------------------------------------------------------------------
# Manuscript A: The Salt Almanac (first person, past tense, literary mystery)
# ---------------------------------------------------------------------------

_SALT_CHAPTERS: list[dict[str, Any]] = [
    {
        "number": 1,
        "title": "The Almanac Room",
        "location": "Almanac Room",
        "summary": "Mira Quill opens the Harrow Isle tide almanac and takes stock of the island.",
        "prose": _p(
            """
            I kept the tide almanac in a room that smelled of salt and cold iron. The pages drank the damp until they went soft at the corners, and each day I opened them with the care of a woman handling a wound. Outside, the Harrow Light stood over the harbor like a white finger, and below it the fishing boats knocked against the quay.

            My eyes are green, like my mother's, and the almanac room was the only place on the island where nobody asked me to look pleasant about it. A scar pulls at the right corner of my jaw when the wind changes. I had got it the winter the Morrow went missing, and I had told three different stories about it since.

            My brother Jonah came in with his coat still shedding rain. The black gull tattooed on his left forearm showed where the sleeve had ridden up, and he grinned at me the way he did when he had already decided something foolish.

            "Constable Rusk wants the harbor ledger again," he said. "He's been at the office since before breakfast, favoring that right leg of his."

            Rusk had taken a ball in the right thigh during the Coxswain's War, and he never let anyone forget which leg it was. I put the almanac on its shelf and counted the coins in my pocket. Seven shillings and sixpence. The Harrow Light drinks one ounce of whale oil an hour, and the oil house had less than a week left.
            """
        ),
        "clock": {"day_index": 1, "time_of_day": 360, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 2,
        "title": "The Gull and the Ledger",
        "location": "Harrow Harbor",
        "summary": "Jonah helps Mira weigh the morning catch and teases her about the almanac.",
        "prose": _p(
            """
            The harbor at six was all rope and breath. Jonah hauled the scale out of the shed while I chalked the weights on the board. He had left his coat in the boat, and the gull on his left forearm showed dark against his skin whenever he reached across the table.

            "You wrote down the tide twice," he said. "Once in the book and once on your cuff."

            I looked down. He was right. I had inked a small blue line on my sleeve without noticing.

            He set the ledger on the scale and grinned at me. "You'll be seeing tides in your sleep before the month is out."

            The fishermen came in one by one with their crates of hake and pollack. Jonah weighed each one and called the numbers, and I wrote them down. Between boats he told me about a stranger who had asked after the old Morrow wreck. He did not know the name, only that the man paid in new coin and asked too many questions about the north channel.

            By noon the rain had stopped. I walked back to the almanac room with the ledger under my arm and the smell of fish on my hands. The Harrow Light was dark, as it should be in daylight, and the island looked almost kind.
            """
        ),
        "clock": {"day_index": 2, "time_of_day": 360, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 3,
        "title": "A Grey Mirror",
        "location": "Harbor Office",
        "summary": "Constable Rusk questions Mira about the Morrow's missing log.",
        "prose": _p(
            """
            Rusk had a way of filling a room without moving. He turned his cap in his hands and looked at the harbor ledger as if it had personally insulted him.

            "Your father kept the Morrow's log," he said. "I want to know where it went."

            "It burned with the rest of the wheelhouse," I said. "You saw the wreck."

            "I saw what was left of it."

            I caught my reflection in the dark window. The harbor lay behind me, grey and flat, and for a moment I did not recognize the woman in the glass.

            Rusk turned his cap again. "There are people on the mainland who would pay well for that log."

            "Then they can pay the sea."

            He smiled without warmth and went out. The bell over the door kept ringing after he had gone. I stood there until the harbor clock struck nine, and then I went back to the almanac room and bolted the door.
            """
        ),
        "clock": {"day_index": 3, "time_of_day": 480, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 4,
        "title": "The Tide Gate",
        "location": "Tide Gate",
        "summary": "Sefa Quill opens the old tide gate while Mira holds the lamp.",
        "prose": _p(
            """
            The tide gate had been built before the lighthouse and had outlived three harbor masters. Sefa set both hands on the wheel and nodded for me to stand back. The iron groaned, and the gate lifted a hand's width, and the trapped water went out through the channel with a sound like a long exhale.

            "There," she said. "Now the eel traps can breathe."

            We went up to the oil house afterward. Sefa filled the reservoir to the mark and trimmed the wick. The Harrow Light drinks one ounce of whale oil an hour, and she measured it out in a brass cup as if she were measuring medicine.

            "Your brother wants to take the north channel tomorrow," she said. "Tell him the tide won't hold him."

            "He won't listen."

            "Then tell him twice."

            I held the lantern while she worked. The flame was small and blue at the root, and it made her white hair look like frost. When the wick was trimmed she set the glass back and wiped her hands on her apron.

            "There are things in this house older than the light," she said. "Don't go prying at them just because Rusk has got you frightened."

            I did not tell her that I was already frightened. I only took the lantern and followed her down the stair.
            """
        ),
        "clock": {"day_index": 4, "time_of_day": 540, "duration_days": 0.25, "precision": "hour"},
    },
    {
        "number": 5,
        "title": "The Charter",
        "location": "Harrow Harbor",
        "summary": "Mira hires a boat to search the north channel and hears a mainland name.",
        "prose": _p(
            """
            The boatman was called Orrin Pike, and he had a face like a boiled boot. He wanted two shillings to take me out to the north channel and back before the weather turned.

            I counted two shillings into his palm. He bit one, which was insulting, and pocketed both.

            "You'll want to be back before the ebb," he said. "The channel gets teeth."

            Rusk said a stranger had been asking about the Morrow. The man had paid in new coin and asked too many questions. I asked what name he had given.

            "A Mr. Vane," Rusk said. "Staying at the Kettle and Gull."

            I knew the name Vane the way I knew the smell of tar. Cade Vane had been harbor master before my father, and he had left the island under a cloud that nobody had ever bothered to name.

            Pike's boat was called the *Mary Bell*, and she smelled of diesel and old rope. We pushed off at noon. The north channel was empty except for the seals and a line of weed that marked the old wreck buoy. I stood at the bow until my eyes watered, and when we came back the light was already turning.
            """
        ),
        "clock": {"day_index": 5, "time_of_day": 720, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 6,
        "title": "The Morrow Comes Home",
        "location": "Shingle Shore",
        "summary": "A storm drives the old wreck onto the shingle and Jonah Quill drowns trying to save a child.",
        "prose": _p(
            """
            The storm came out of the northwest at dusk, and by midnight the sea was breaking over the quay. I was in the almanac room when I heard the bell. Jonah was already on the shingle with a rope around his chest.

            The Morrow's ribs had come up out of the sand. A child from the Kettle and Gull had gone down to look at them and got caught in the backwash. Jonah went in after her.

            He got her to the rope. I saw his hand close on it. Then the next wave took him and the sea did not give him back.

            ***

            We found him when the tide turned, tangled in the weed line a hundred yards east. Sefa identified him because I could not make my mouth work. We wrapped him in a grey sail and laid him in the oil house until the coffin could be brought over.

            Tamsin kept the harbor ledger under her arm while the men carried him up. She did not say anything. She only stood at the door with the rain running off her hat and watched the shingle as if she expected the sea to apologize.

            Sefa filled the reservoir at dusk, and the light burned all night. I sat with her in the almanac room and did not open the book.
            """
        ),
        "clock": {"day_index": 6, "time_of_day": 1080, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 7,
        "title": "The Oil House",
        "location": "Oil House",
        "summary": "Sefa keeps the light burning while Mira tries to write the almanac entry for Jonah's death.",
        "prose": _p(
            """
            By mid-morning the whole island knew. The oil house was warm and close and smelled of the sea's own fat. Sefa filled the reservoir to the mark, trimmed the wick, and set the glass back with the careful violence of a woman who had done it ten thousand times.

            "Write it down," she said. "If you don't write it down, you'll carry it in your head, and your head has enough."

            I opened the almanac to the page for the sixth of March. The pen shook. I wrote *Jonah Quill, lost in the north channel*, and then I could not think of anything else to say.

            Tamsin came in with the harbor ledger and set it on the table. She had been crying, but she had stopped. "The board wants an accounting," she said. "They want to know what the Morrow was carrying."

            "It was carrying nothing," I said. "It was a wreck."

            "That's what I told them."

            She looked at the ledger and then at me, and for a moment I thought she was going to say something else. Then she shook her head and went out.

            Sefa took the pen from my hand and finished the entry in her small, upright script. The light turned above us, slow and patient, and the island settled into the rest of its life.
            """
        ),
        "clock": {"day_index": 7, "time_of_day": 600, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 8,
        "title": "The Scar",
        "location": "Shingle Shore",
        "summary": "Mira confronts Rusk on the shingle and learns that Tamsin has been talking.",
        "prose": _p(
            """
            Rusk was waiting for me where the shingle turned to sand. The wind cut at my jaw, and I pulled my collar up.

            "You've been asking about the Morrow," he said. "That's a dangerous habit for a woman with a ledger."

            "It's my ledger."

            "It was your father's ledger."

            Tamsin waited for me under the oil-house eaves. She had a way of standing very still that made her look younger than she was. When Rusk had gone she came down the shingle and put her hand on my arm.

            "He's been asking about the wreck since before the storm," she said. "He came to the office three times last week."

            "What did you tell him?"

            "That I keep the harbor ledger and not the Morrow's."

            I looked out at the water. The wreck had gone back under, all but a few ribs, and the gulls were already using them for perches.

            "Tamsin," I said. "If you know something about my father, you should tell me now."

            She took her hand away. "I know what the harbor knows," she said. "And the harbor doesn't say it out loud."

            She went back up the shingle. I stayed until Sefa lit the beacon with the evening measure, and then I went home.
            """
        ),
        "clock": {"day_index": 8, "time_of_day": 1020, "duration_days": 0.25, "precision": "hour"},
    },
    {
        "number": 9,
        "title": "The Telescope",
        "location": "Chandlery",
        "summary": "Mira prices a brass telescope she cannot afford and is followed by Cade Vane.",
        "prose": _p(
            """
            The chandlery kept a brass telescope in the window, and I had been looking at it for three years. It was a Hensley, with a leather tube and a lens that made the far islands look close enough to touch.

            Forty guineas was more than I would see in a year. I turned it over and put it back on the felt.

            The shop bell rang. I looked up, but the doorway was empty except for the rain.

            Vane stood under the chandlery awning when I came out. He had a new coat and a walking stick with a silver head, and he smiled at me as if we were old friends.

            "Miss Quill," he said. "I hear you keep the almanac now."

            "Someone has to."

            "Your father kept it before you. He was a careful man with other people's secrets."

            I did not answer. He looked past me at the harbor, and for a moment the smile went out of his face.

            "If you find anything of his," he said, "anything at all, I would pay handsomely to see it."

            I walked away without saying goodbye. The rain had got into my collar, and the telescope in the window watched me go with its one dark eye.
            """
        ),
        "clock": {"day_index": 9, "time_of_day": 900, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 10,
        "title": "The Tide Bell",
        "location": "Beacon Gallery",
        "summary": "Rusk lights the beacon, Vane warns Mira off the almanac, and Sefa finally tells her the tide-gate combination.",
        "prose": _p(
            """
            The tide bell rang at eight, and Rusk climbed the beacon stair with the oil can in his hand. He filled the reservoir and trimmed the wick. The flame caught on the first match and settled into its slow white hum.

            "You should let me do that," I said.

            "You've got enough to do."

            Vane found me at the foot of the beacon stair. He did not have his walking stick this time, and without it he looked smaller.

            "Those almanac codes," he said. "They're just weather and tides. They won't stand up in any court."

            "I never said they would."

            "Good."

            He went out into the dark. I stood in the stairwell until the bell rang again.

            Sefa was waiting for me in the almanac room. She had the tide-gate key on the table and a look on her face I had not seen since I was a child.

            "You'll need the gate tomorrow," she said. "The combination is four, nine, two. Say it back."

            "Four, nine, two."

            "Good. Now write it down somewhere you won't lose it."

            I wrote it in the back of the almanac, under the tide tables, where my father used to keep his private notes.
            """
        ),
        "clock": {"day_index": 10, "time_of_day": 480, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 11,
        "title": "The Left Leg",
        "location": "Shingle Shore",
        "summary": "Mira walks the shore at night and thinks about the shape of the island's silence.",
        "prose": _p(
            """
            The harbor clock read 11:20 when I started down the shingle. The tide was out, and the wreck ribs stood up out of the wet sand like the fingers of a hand.

            I walked the shore alone. The light behind me turned the weed line silver, and the water made a small, persistent sound against the stones.

            At half past midnight I turned back. Rusk came down the shingle favoring his right leg. He had his coat buttoned wrong, as if he had dressed in the dark.

            "You're out late," he said.

            "So are you."

            "I couldn't sleep."

            We stood there for a while. The wind came off the water and made my eyes water.

            "Your father was a good man," Rusk said. "I want you to know that."

            "You didn't act like it when he was alive."

            "No," he said. "I didn't."

            He went on down the shingle, and I watched him until the dark took him. Then I went home and bolted the door.
            """
        ),
        "clock": {"day_index": 11, "time_of_day": 1400, "duration_days": 0.25, "precision": "hour"},
    },
    {
        "number": 12,
        "title": "The Reward",
        "location": "Harbor Office",
        "summary": "The harbor board pays Mira for the Morrow inquiry, and Sefa warns her about Vane.",
        "prose": _p(
            """
            The harbor board met in the back room of the Kettle and Gull and voted to pay me ten shillings for the Morrow inquiry. It was not charity. They wanted the wreck inventoried before the spring tides took it, and I was the only person on the island who could read my father's hand.

            The clerk counted the coins onto the table. I put them in my pocket and signed the receipt.

            Rusk was waiting in the doorway with a sealed letter. He broke the wax, read it twice, and folded it into his coat.

            "Aurel Vane," he said. "I might have known."

            Sefa was waiting outside. She had her shawl around her shoulders and her cane hooked over her arm.

            "They'll want more than an inventory before they're done," she said. "Vane has been buying drinks for the board."

            "Let him."

            "Mira."

            "I know."

            We walked back along the quay. The light was turning, and the first gulls were coming in to roost on the warehouse roofs.

            "Your father kept two ledgers," Sefa said. "One for the harbor and one for himself. If Vane finds the second one before you do, you won't have to worry about the board."

            "Where is it?"

            "I don't know. He never told me."

            She said it too quickly. I did not press her. We walked the rest of the way in silence.
            """
        ),
        "clock": {"day_index": 12, "time_of_day": 600, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 13,
        "title": "The Third Morning",
        "location": "Archive",
        "summary": "Tamsin discovers that the harbor ledger was falsified, and Mira rows out to the wreck alone.",
        "prose": _p(
            """
            Tamsin set the ledger on the table and opened it to the third page. The archive was cold, and the lantern between us made a small yellow room out of the dark.

            "Look at the ink," she said. "The March entries were written in one sitting. The dates are wrong."

            I leaned over the page. The numbers swam, and then they settled.

            "This ledger was falsified," Tamsin said. "Someone copied the harbor accounts after the Morrow went down."

            "Can you prove it?"

            "I can prove the ink is newer than the paper."

            I pulled the oars alone. The *Mary Bell* was heavier than she looked, and the north channel was full of small contrary currents. The wreck came up out of the water slowly, like a thought I had been trying not to have.

            I stayed out until the light began to fail. When I came back, Sefa was on the quay with a lantern and a face like a closed door.
            """
        ),
        "clock": {"day_index": 13, "time_of_day": 660, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 14,
        "title": "The Cargo",
        "location": "Harrow Harbor",
        "summary": "A fisherman lets slip what the Morrow carried, and Sefa keeps the light through the long dusk.",
        "prose": _p(
            """
            The fisherman was called Neddy Croft, and he had been drunk since the wreck. He sat on the capstan with a bottle between his knees and watched the light come on.

            "The Morrow," he said. "She wasn't carrying fish."

            "What was she carrying?"

            "That's the question, isn't it."

            The fisher spat and said the Morrow carried medical glass and whale oil. That was the story the harbor board had put about, and most of the island had accepted it.

            I left him on the capstan and went up to the oil house. Sefa refilled the reservoir at dusk. The light turned above us, one slow revolution after another, and the sea went from grey to black.

            "You should sleep," she said.

            "I will."

            "You won't."

            She was right. I sat in the almanac room until the small hours, reading my father's tide tables and trying to find the second ledger in the spaces between the numbers.
            """
        ),
        "clock": {"day_index": 14, "time_of_day": 1140, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 15,
        "title": "The Cistern",
        "location": "Cistern",
        "summary": "Vane's man finds the almanac's hiding place, and Mira understands what her father buried.",
        "prose": _p(
            """
            The cistern stood behind the church, a round brick mouth in a field of nettles. Four afternoons ago, Sefa and I had sealed the cistern. Now the seal was broken, and a man I did not know was standing at the edge with a lantern.

            I stayed behind the hedge and watched. Vane came up the path a few minutes later, and the man handed him something wrapped in oilcloth.

            "Under the cistern," the man said. "Just where you said."

            Vane turned the packet over. Even from the hedge I could see the shape of it: a book, bound in green cloth, with a brass clasp.

            "My man found it under the cistern," Vane said to the dark. "The almanac was there all along."

            I did not move. The nettles were wet, and my boots were filling with water, and I could hear my own breath loud in my ears.

            Vane put the packet inside his coat and walked back toward the harbor. The man followed with the lantern. I stayed behind the hedge until the light had gone, and then I went to find Sefa.
            """
        ),
        "clock": {"day_index": 15, "time_of_day": 600, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 16,
        "title": "The Last Tide",
        "location": "Kettle and Gull",
        "summary": "Mira chooses what to do with the almanac, and Rusk pays his debt.",
        "prose": _p(
            """
            The Kettle and Gull was almost empty. Sefa had gone to bed, and the fire was down to its last red eye. I set a third cup on the table out of habit. Then I sat looking at it until the landlord came over and asked if I wanted another pot.

            "No," I said. "Leave the cup."

            Rusk came in at closing. He had a purse in his hand and a look on his face that I had not seen before.

            "For the inquiry," he said. "The board voted it, and I said I'd bring it."

            Rusk counted two shillings onto the table. I did not touch them.

            "You knew," I said. "About my father. About the Morrow."

            "I knew enough."

            "And you let me find out from Tamsin."

            He sat down without being asked. The chair creaked under him. "I was going to tell you," he said. "Then the storm came, and Jonah, and I thought maybe you'd had enough."

            "I don't get to have enough."

            He nodded as if that were the only answer he had expected. We sat there until the fire went out. Then he counted the coins again, though they had not moved, and wished me good night.
            """
        ),
        "clock": {"day_index": 16, "time_of_day": 1380, "duration_days": 0.25, "precision": "hour"},
    },
]

_SALT_CHARACTERS: list[dict[str, Any]] = [
    {
        "name": "Mira Quill",
        "aliases": ["Mira", "the almanac keeper"],
        "role": "protagonist",
        "summary": "Keeper of the Harrow Isle tide almanac and daughter of the old lighthouse keeper.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "wry, watchful",
            "vocabulary": ["tide", "ledger", "salt"],
            "sentence_structure": ["close first person", "concrete detail"],
            "tics": ["counts things when frightened"],
            "forbidden_words": [],
            "example_lines": ["The light drinks one ounce an hour.", "I write it down so I don't carry it."],
        },
        "emotional_profile": {
            "baseline_mood": "guarded",
            "triggers": ["the Morrow", "being pitied"],
            "coping_mechanisms": ["work", "notation"],
            "core_wound": "her brother's death and her father's silence",
        },
    },
    {
        "name": "Jonah Quill",
        "aliases": ["Jonah"],
        "role": "supporting",
        "summary": "Mira's older brother, a fisherman and occasional smuggler.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "easy, teasing",
            "vocabulary": ["boat", "weather", "tide"],
            "sentence_structure": ["short declaratives"],
            "tics": ["grins before bad news"],
            "forbidden_words": [],
            "example_lines": ["You'll be seeing tides in your sleep.", "Tell her the tide won't hold."],
        },
        "emotional_profile": {
            "baseline_mood": "restless",
            "triggers": ["being cornered", "his father's name"],
            "coping_mechanisms": ["jokes", "risk"],
            "core_wound": "he never got his father's approval",
        },
    },
    {
        "name": "Sefa Quill",
        "aliases": ["Sefa", "Aunt Sefa"],
        "role": "supporting",
        "summary": "Mira's aunt, the island's senior lighthouse keeper and keeper of its older secrets.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "dry, deliberate",
            "vocabulary": ["wick", "reservoir", "weather"],
            "sentence_structure": ["plain instruction"],
            "tics": ["cleans her hands before saying something hard"],
            "forbidden_words": [],
            "example_lines": ["Fill it to the mark.", "There are things older than the light."],
        },
        "emotional_profile": {
            "baseline_mood": "steady",
            "triggers": ["Rusk", "the cistern"],
            "coping_mechanisms": ["routine", "silence"],
            "core_wound": "she promised her brother she would keep the secret",
        },
    },
    {
        "name": "Ivo Rusk",
        "aliases": ["Rusk", "Constable Rusk"],
        "role": "antagonist",
        "summary": "Harrow Isle's constable, a war veteran who wants the Morrow's missing log.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "flat, needling",
            "vocabulary": ["law", "ledger", "evidence"],
            "sentence_structure": ["questions as statements"],
            "tics": ["turns his cap"],
            "forbidden_words": [],
            "example_lines": ["I saw what was left of it.", "That's a dangerous habit."],
        },
        "emotional_profile": {
            "baseline_mood": "suspicious",
            "triggers": ["being lied to", "the Coxswain's War"],
            "coping_mechanisms": ["procedure"],
            "core_wound": "he obeyed an order he knew was wrong",
        },
    },
    {
        "name": "Tamsin Rill",
        "aliases": ["Tamsin"],
        "role": "supporting",
        "summary": "Harbor clerk who keeps the island's official ledger and its unofficial worries.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "quiet, exact",
            "vocabulary": ["ink", "paper", "board"],
            "sentence_structure": ["qualified statements"],
            "tics": ["holds ledgers against her chest"],
            "forbidden_words": [],
            "example_lines": ["The board wants an accounting.", "The ink is newer than the paper."],
        },
        "emotional_profile": {
            "baseline_mood": "anxious",
            "triggers": ["Rusk", "being asked to choose"],
            "coping_mechanisms": ["order", "small corrections"],
            "core_wound": "she signed a false entry years ago",
        },
    },
    {
        "name": "Cade Vane",
        "aliases": ["Vane", "Mr. Vane"],
        "role": "antagonist",
        "summary": "Former harbor master and mainland fixer with a long claim on the Morrow.",
        "realm": "Harrow Isle",
        "voice_profile": {
            "tone": "smooth, threatening",
            "vocabulary": ["arrangement", "courtesy", "hands"],
            "sentence_structure": ["long courtesies"],
            "tics": ["smiles before a threat"],
            "forbidden_words": [],
            "example_lines": ["I would pay handsomely to see it.", "He was a careful man with other people's secrets."],
        },
        "emotional_profile": {
            "baseline_mood": "controlled",
            "triggers": ["exposure", "the almanac"],
            "coping_mechanisms": ["money", "manners"],
            "core_wound": "he lost the harbor and never forgave the man who took it",
        },
    },
]

_SALT_FACTS: list[dict[str, Any]] = [
    {
        "fact_type": "character_attribute",
        "key": "Mira eye color",
        "value": "green",
        "subject": "Mira Quill",
        "predicate": "Mira eye color",
        "value_kind": "text",
        "value_text": "green",
    },
    {
        "fact_type": "character_attribute",
        "key": "Mira jaw scar",
        "value": "right jaw",
        "subject": "Mira Quill",
        "predicate": "Mira jaw scar",
        "value_kind": "text",
        "value_text": "right jaw",
    },
    {
        "fact_type": "character_attribute",
        "key": "Jonah Quill tattoo",
        "value": "black gull on the left forearm",
        "subject": "Jonah Quill",
        "predicate": "Jonah Quill tattoo",
        "value_kind": "text",
        "value_text": "black gull on the left forearm",
    },
    {
        "fact_type": "character_attribute",
        "key": "Ivo Rusk limp",
        "value": "right leg",
        "subject": "Ivo Rusk",
        "predicate": "Ivo Rusk limp",
        "value_kind": "text",
        "value_text": "right leg",
    },
]

_SALT_SECRETS: list[dict[str, Any]] = [
    {
        "fact_type": "secret_identity",
        "key": "morrow_scuttling",
        "value": "Elias Quill scuttled the Morrow for Cade Vane",
        "subject": "Mira Quill",
        "predicate": "morrow_scuttling",
        "value_kind": "text",
        "value_text": "Elias Quill scuttled the Morrow for Cade Vane",
        "secrecy": {
            "holders": ["Mira Quill", "Sefa Quill"],
            "note": "Only Mira and Sefa know that Elias Quill opened the Morrow's sea-cocks on Vane's orders.",
        },
    },
    {
        "fact_type": "secret_code",
        "key": "almanac_opium_codes",
        "value": "Vane smuggled opium under the night tide",
        "subject": "Mira Quill",
        "predicate": "almanac_opium_codes",
        "value_kind": "text",
        "value_text": "Vane smuggled opium under the night tide",
        "secrecy": {
            "holders": ["Mira Quill", "Sefa Quill"],
            "note": "The almanac's tide codes record Vane's opium runs; only Mira and Sefa can read them.",
        },
    },
]

_SALT_KNOWLEDGE: list[dict[str, Any]] = [
    {
        "character": "Mira Quill",
        "fact": "the tide-gate combination is four, nine, two",
        "source_summary": "Sefa gave Mira the combination in Chapter 10.",
        "learned_at": {"book_number": 1, "chapter_number": 10, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["tide gate"],
        "reader_visible": True,
    },
    {
        "character": "Ivo Rusk",
        "fact": "the mainland charter party is led by Aurel Vane",
        "source_summary": "Rusk received a sealed letter naming Aurel Vane in Chapter 12.",
        "learned_at": {"book_number": 1, "chapter_number": 12, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["Vane", "charter"],
        "reader_visible": True,
    },
    {
        "character": "Tamsin Rill",
        "fact": "the harbor ledger was falsified after the Morrow went down",
        "source_summary": "Tamsin compared the ink and paper in Chapter 13.",
        "learned_at": {"book_number": 1, "chapter_number": 13, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["ledger", "forgery"],
        "reader_visible": True,
    },
    {
        "character": "Cade Vane",
        "fact": "the almanac was hidden under the cistern",
        "source_summary": "Vane's man found the packet under the cistern in Chapter 15.",
        "learned_at": {"book_number": 1, "chapter_number": 15, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["almanac", "cistern"],
        "reader_visible": True,
    },
]

_SALT_WORLD_RULES: list[dict[str, Any]] = [
    {
        "rule_name": "Harrow Light Oil Toll",
        "rule_type": "technology_constraint",
        "description": (
            "The Harrow Light burns one measured ounce of whale oil each hour. The wick must be "
            "trimmed and the reservoir filled before lighting; a dry wick cannot hold a flame, and "
            "the light cannot burn without oil."
        ),
        "scan_pattern": r"dry wick|empty reservoir|without oil|without another drop|three days without",
        "relevance_tags": ["lighthouse", "oil", "cost"],
        "established_in": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": None},
    },
]

_SALT_CALENDAR: dict[str, Any] = {
    "days_per_week": 7,
    "hours_per_day": 24,
    "week_day_names": [],
    "months": [],
    "days_per_year": 360,
    "epoch_label": "Harrow Year 1",
}

_SALT_QUANTITY_SCHEME: dict[str, Any] = {
    "measure": "coin",
    "denominations": [
        {"name": "penny", "per_base": 1},
        {"name": "shilling", "per_base": 12},
        {"name": "pound", "per_base": 240},
        {"name": "guinea", "per_base": 252},
    ],
    "bands": [
        {"name": "destitute", "lower_bound": 0},
        {"name": "scraping", "lower_bound": 24},
        {"name": "modest", "lower_bound": 240},
        {"name": "comfortable", "lower_bound": 1200},
        {"name": "wealthy", "lower_bound": 6000},
        {"name": "princely", "lower_bound": 30000},
    ],
    "max_band_jump": 1,
}

_SALT_QUANTITY_CLEAN: list[dict[str, Any]] = [
    {
        "character": "Mira Quill",
        "chapter": 2,
        "scene_order": 1,
        "amount": 90.0,
        "unit": "penny",
        "band": "scraping",
        "change_reason": "opening balance",
    },
    {
        "character": "Ivo Rusk",
        "chapter": 2,
        "scene_order": 1,
        "amount": 480.0,
        "unit": "penny",
        "band": "modest",
        "change_reason": "opening balance",
    },
    {
        "character": "Mira Quill",
        "chapter": 5,
        "scene_order": 1,
        "amount": 66.0,
        "unit": "penny",
        "band": "scraping",
        "change_reason": "paid Orrin Pike two shillings for the charter",
    },
    {
        "character": "Mira Quill",
        "chapter": 10,
        "scene_order": 1,
        "amount": 126.0,
        "unit": "penny",
        "band": "scraping",
        "change_reason": "piloting fee from the harbor board",
    },
    {
        "character": "Mira Quill",
        "chapter": 12,
        "scene_order": 1,
        "amount": 246.0,
        "unit": "penny",
        "band": "modest",
        "change_reason": "harbor board inquiry payment",
    },
    {
        "character": "Ivo Rusk",
        "chapter": 16,
        "scene_order": 1,
        "amount": 456.0,
        "unit": "penny",
        "band": "modest",
        "change_reason": "paid two shillings for the Morrow inquiry",
    },
]

_SALT_QUANTITY_PLANTED: list[dict[str, Any]] = [
    row if not (row["character"] == "Mira Quill" and row["chapter"] == 12) else {
        "character": "Mira Quill",
        "chapter": 12,
        "scene_order": 1,
        "amount": 9000.0,
        "unit": "penny",
        "band": "wealthy",
        "change_reason": None,
    }
    for row in _SALT_QUANTITY_CLEAN
]

_SALT_DEATHS: list[dict[str, Any]] = [
    {
        "character": "Jonah Quill",
        "chapter": 6,
        "scene_order": 1,
        "status": ["dead", "deceased"],
        "notes": ["Drowned saving a child from the Morrow's backwash in Chapter 6."],
        "source_summary": "Jonah Quill drowned at the Morrow wreck.",
    },
]

_SALT_PLANTS: list[dict[str, Any]] = [
    _plant(
        "salt.character_attribute.tattoo",
        "character_attribute",
        "unkeyed_prose_contradiction",
        2,
        "Jonah's gull tattoo is described on his right forearm instead of the canonical left forearm.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("jonah quill", "tattoo", "gull", "right forearm", "left forearm"),
        anchor="He set the ledger on the scale and grinned at me.",
        text='Jonah rolled up his right sleeve, and the black gull surfaced above his wrist.',
        fact_key="Jonah Quill tattoo",
        clean_forbidden=("right sleeve",),
    ),
    _plant(
        "salt.character_attribute.eyes",
        "character_attribute",
        "lexical_key_mention",
        3,
        "Mira's eyes are described as grey instead of the canonical green.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("mira eye color", "mira quill", "grey", "green"),
        anchor="I caught my reflection in the dark window.",
        text="My eyes looked back grey as the harbor, not their usual green.",
        fact_key="Mira eye color",
        clean_forbidden=("grey as the harbor",),
    ),
    _plant(
        "salt.character_attribute.scar",
        "character_attribute",
        "unkeyed_prose_contradiction",
        8,
        "Mira's jaw scar is placed on her left cheek instead of the canonical right jaw.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("mira quill", "scar", "left cheek", "right jaw"),
        anchor="The wind cut at my jaw, and I pulled my collar up.",
        text="The old scar on my left cheek throbbed.",
        fact_key="Mira jaw scar",
        clean_forbidden=("left cheek",),
    ),
    _plant(
        "salt.character_attribute.limp",
        "character_attribute",
        "unkeyed_prose_contradiction",
        11,
        "Rusk's limp is described in his left leg instead of the canonical right leg.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("rusk", "limp", "left leg", "right leg"),
        anchor="Rusk came down the shingle favoring his right leg.",
        text="His left leg dragged in the shingle behind him.",
        fact_key="Ivo Rusk limp",
        clean_forbidden=("left leg dragged",),
    ),
    _plant(
        "salt.dead_character_acting.jonah_ch9",
        "dead_character_acting",
        "post_mortem_dialogue",
        9,
        "Jonah Quill speaks in the chandlery three chapters after drowning in Chapter 6.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("jonah quill", "dead", "deceased"),
        anchor="The shop bell rang.",
        text='"You always did like the brass ones," Jonah said from the doorway.',
        death_chapter=6,
        clean_forbidden=("Jonah said",),
    ),
    _plant(
        "salt.dead_character_acting.jonah_ch11",
        "dead_character_acting",
        "post_mortem_action",
        11,
        "Jonah Quill walks the shore beside Mira after drowning in Chapter 6.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("jonah quill", "dead", "deceased"),
        anchor="I walked the shore alone.",
        text="Jonah walked beside me, his boots leaving no mark.",
        death_chapter=6,
        clean_forbidden=("Jonah walked",),
    ),
    _plant(
        "salt.dead_character_acting.jonah_ch13",
        "dead_character_acting",
        "post_mortem_action",
        13,
        "Jonah Quill rows the boat with Mira after drowning in Chapter 6.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("jonah quill", "dead", "deceased"),
        anchor="I pulled the oars alone.",
        text="Jonah sat across from me, rowing with the old easy rhythm.",
        death_chapter=6,
        clean_forbidden=("Jonah sat across",),
    ),
    _plant(
        "salt.dead_character_acting.jonah_ch16",
        "dead_character_acting",
        "post_mortem_dialogue_and_action",
        16,
        "Jonah Quill laughs and pours tea at the Kettle and Gull after drowning in Chapter 6.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("jonah quill", "dead", "deceased"),
        anchor="I set a third cup on the table out of habit.",
        text="Jonah laughed and poured the tea.",
        death_chapter=6,
        clean_forbidden=("Jonah laughed",),
    ),
    _plant(
        "salt.timeline_chronology_rewind.clock_ch8",
        "timeline_chronology_rewind",
        "inter_scene_clock_rewind",
        8,
        "Chapter 8's linear scene clock rewinds to day 5 after Chapter 7 at day 7.",
        ("chronology", "temporal_coherence"),
        ("chapter 8", "earlier in story time", "day 5"),
        kind="clock",
        clock_override={"day_index": 5, "time_of_day": 1020, "duration_days": 0.25, "precision": "hour"},
    ),
    _plant(
        "salt.timeline_chronology_rewind.intra_ch11",
        "timeline_chronology_rewind",
        "intra_scene_meridian_rewind",
        11,
        "Chapter 11 jumps from 11:20 p.m. back to 9:15 the same evening without a flashback marker.",
        ("temporal_coherence", "chronology"),
        ("9:15", "11:20", "scene:salt-11"),
        anchor="The harbor clock read 11:20 when I started down the shingle.",
        text="At a quarter past nine that evening, I reached the tide line.",
        clean_forbidden=("quarter past nine",),
    ),
    _plant(
        "salt.timeline_chronology_rewind.clock_ch13",
        "timeline_chronology_rewind",
        "inter_scene_clock_rewind",
        13,
        "Chapter 13's linear scene clock rewinds to day 9 after Chapter 12 at day 12.",
        ("chronology", "temporal_coherence"),
        ("chapter 13", "earlier in story time", "day 9"),
        kind="clock",
        clock_override={"day_index": 9, "time_of_day": 660, "duration_days": 0.5, "precision": "hour"},
    ),
    _plant(
        "salt.timeline_chronology_rewind.relative_ch15",
        "timeline_chronology_rewind",
        "relative_time_rewind",
        15,
        "Chapter 15 calls the cistern sealing 'the previous afternoon' after Chapter 12 established it four afternoons earlier.",
        ("temporal_coherence", "chronology"),
        ("previous afternoon", "four afternoons ago", "cistern"),
        anchor="Four afternoons ago, Sefa and I had sealed the cistern.",
        text="The previous afternoon, we had sealed it.",
        clean_forbidden=("previous afternoon",),
    ),
    _plant(
        "salt.knowledge_leak_before_learned.combination",
        "knowledge_leak_before_learned",
        "exact_fact_leak",
        4,
        "Mira uses the tide-gate combination in Chapter 4, six chapters before Sefa teaches it to her.",
        ("knowledge_timing",),
        ("mira quill", "tide-gate", "four, nine, two", "chapter 10"),
        anchor="Sefa set both hands on the wheel and nodded for me to stand back.",
        text="I reached past her and spun the tide-gate dial to four, nine, two.",
        learned_at_chapter=10,
        clean_forbidden=("four, nine, two",),
    ),
    _plant(
        "salt.knowledge_leak_before_learned.vane_name",
        "knowledge_leak_before_learned",
        "paraphrased_leak",
        5,
        "Rusk names Aurel Vane in Chapter 5 before receiving the sealed letter in Chapter 12.",
        ("knowledge_timing",),
        ("rusk", "aurel vane", "chapter 12"),
        anchor="Rusk said a stranger had been asking about the Morrow.",
        text='"Aurel Vane paid for the crossing," Rusk said.',
        learned_at_chapter=12,
        clean_forbidden=("aurel vane",),
    ),
    _plant(
        "salt.knowledge_leak_before_learned.ledger",
        "knowledge_leak_before_learned",
        "exact_fact_leak",
        6,
        "Tamsin says the harbor ledger was falsified in Chapter 6, before she proves it in Chapter 13.",
        ("knowledge_timing",),
        ("tamsin", "ledger", "falsified", "chapter 13"),
        anchor="Tamsin kept the harbor ledger under her arm while the men carried him up.",
        text='"The harbor ledger was falsified," Tamsin said.',
        learned_at_chapter=13,
        clean_forbidden=("ledger was falsified",),
    ),
    _plant(
        "salt.knowledge_leak_before_learned.cistern",
        "knowledge_leak_before_learned",
        "paraphrased_leak",
        9,
        "Vane says the almanac is under the cistern in Chapter 9, before his man finds it in Chapter 15.",
        ("knowledge_timing",),
        ("vane", "cistern", "almanac", "chapter 15"),
        anchor="Vane stood under the chandlery awning when I came out.",
        text='"She hid the almanac under the cistern," Vane said.',
        learned_at_chapter=15,
        clean_forbidden=("under the cistern",),
    ),
    _plant(
        "salt.world_rule_cost_violation.dry_lamp_ch4",
        "world_rule_cost_violation",
        "explicit_cost_omission",
        4,
        "A lamp is lit with a dry wick and catches at once, bypassing the Harrow Light oil toll.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "harrow light", "dry wick", "oil"),
        anchor="Sefa filled the reservoir to the mark and trimmed the wick.",
        text="I lit the spare lamp with a dry wick, and the flame caught at once.",
        clean_forbidden=("dry wick",),
    ),
    _plant(
        "salt.world_rule_cost_violation.empty_reservoir_ch7",
        "world_rule_cost_violation",
        "implicit_cost_omission",
        7,
        "The Harrow Light burns all night on an empty reservoir, contrary to its measured oil cost.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "harrow light", "empty reservoir"),
        anchor=(
            "Sefa filled the reservoir to the mark, trimmed the wick, and set the glass back with "
            "the careful violence of a woman who had done it ten thousand times."
        ),
        text="The lamp burned until dawn on an empty reservoir, and no one thought to fill it.",
        clean_forbidden=("empty reservoir",),
    ),
    _plant(
        "salt.world_rule_cost_violation.dry_wick_ch10",
        "world_rule_cost_violation",
        "explicit_cost_omission",
        10,
        "Rusk lights the beacon with a dry wick and no oil, contrary to the Harrow Light oil toll.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "harrow light", "dry wick", "without oil"),
        anchor="He filled the reservoir and trimmed the wick.",
        text="Rusk lit the beacon with a dry wick, and the flame took without oil.",
        clean_forbidden=("without oil",),
    ),
    _plant(
        "salt.world_rule_cost_violation.three_days_ch14",
        "world_rule_cost_violation",
        "implicit_cost_omission",
        14,
        "The beacon burns for three days without refilling, contrary to its one-ounce-an-hour cost.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "harrow light", "three days without"),
        anchor="Sefa refilled the reservoir at dusk.",
        text="The beacon burned for three days without another drop.",
        clean_forbidden=("three days without",),
    ),
    _plant(
        "salt.quantity_affordability.charter",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        5,
        "Mira pays three pounds for a charter while her tracked purse holds sixty-six pence.",
        ("affordability", "quantity_drift"),
        ("mira quill", "three pounds", "720", "charter"),
        anchor="I counted two shillings into his palm.",
        text="Then I added three pounds for the day's hire.",
        clean_forbidden=("three pounds",),
    ),
    _plant(
        "salt.quantity_affordability.telescope",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        9,
        "Mira buys a forty-guinea telescope while her tracked purse holds sixty-six pence.",
        ("affordability", "quantity_drift"),
        ("mira quill", "forty guineas", "10080", "telescope"),
        anchor="Forty guineas was more than I would see in a year.",
        text="I counted out forty guineas and carried the telescope home.",
        clean_forbidden=("counted out forty guineas",),
    ),
    _plant(
        "salt.quantity_affordability.band_jump",
        "quantity_affordability",
        "unexplained_band_jump",
        12,
        "Mira's tracked coin jumps from scraping to wealthy with no change_reason.",
        ("quantity_drift", "affordability"),
        ("mira quill", "scraping", "wealthy", "9000"),
        kind="quantity",
        quantity_state={
            "character": "Mira Quill",
            "chapter": 12,
            "scene_order": 1,
            "amount": 9000.0,
            "unit": "penny",
            "band": "wealthy",
            "change_reason": None,
        },
    ),
    _plant(
        "salt.quantity_affordability.rusk_reward",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        16,
        "Rusk pays twenty-five pounds while his tracked purse holds four hundred fifty-six pence.",
        ("affordability", "quantity_drift"),
        ("rusk", "twenty-five pounds", "6000", "reward"),
        anchor="Rusk counted two shillings onto the table.",
        text="Rusk counted twenty-five pounds onto the table.",
        clean_forbidden=("twenty-five pounds",),
    ),
    _plant(
        "salt.secret_leak.father",
        "secret_leak",
        "attributed_dialogue_leak",
        3,
        "Out-of-circle Rusk states that Elias Quill put the Morrow on the rocks.",
        ("secret_leak",),
        ("rusk", "morrow", "father", "scuttled"),
        anchor="Rusk turned his cap again.",
        text='"Your father put the Morrow on the rocks," Rusk said.',
        clean_forbidden=("put the Morrow on the rocks",),
    ),
    _plant(
        "salt.secret_leak.sea_cocks",
        "secret_leak",
        "attributed_dialogue_leak",
        8,
        "Out-of-circle Tamsin says Elias Quill opened the Morrow's sea-cocks.",
        ("secret_leak",),
        ("tamsin", "elias", "sea-cocks", "morrow"),
        anchor="Tamsin waited for me under the oil-house eaves.",
        text='"Elias opened the sea-cocks himself," Tamsin said.',
        clean_forbidden=("opened the sea-cocks",),
    ),
    _plant(
        "salt.secret_leak.opium_vane",
        "secret_leak",
        "attributed_dialogue_leak",
        10,
        "Out-of-circle Vane refers to the almanac's opium-smuggling codes in Chapter 10.",
        ("secret_leak",),
        ("vane", "opium", "almanac", "codes"),
        anchor="Vane found me at the foot of the beacon stair.",
        text='"Those almanac codes won\'t prove I ran opium," Vane said.',
        clean_forbidden=("ran opium",),
    ),
    _plant(
        "salt.secret_leak.opium_fisher",
        "secret_leak",
        "unquoted_narration_leak",
        14,
        "An out-of-circle fisherman says in narration that the Morrow carried opium.",
        ("secret_leak",),
        ("fisher", "opium", "morrow", "cargo"),
        anchor="The fisher spat and said the Morrow carried medical glass and whale oil.",
        text='"Everyone knows the Morrow carried opium," the fisher said.',
        clean_forbidden=("carried opium",),
    ),
]


# ---------------------------------------------------------------------------
# Manuscript B: The Long Orbit of Small Debts (third person, present tense,
# near-future orbital salvage drama)
# ---------------------------------------------------------------------------

_VESTA_CHAPTERS: list[dict[str, Any]] = [
    {
        "number": 1,
        "title": "Station Time",
        "location": "Quartermaster's Office",
        "summary": "Ilya Petrova opens the quartermaster's ledger and takes stock of Vesta Station.",
        "prose": _p(
            """
            At 06:00 station time, Ilya Petrova opens the quartermaster's ledger and finds that the station has spent more than it earned for the eleventh day in a row. This is not a surprise. Vesta Station is a salvage depot bolted to the underside of a dead moon, and salvage depots are not famous for their margins.

            Ilya's left arm is a matte-black prosthetic from shoulder to wrist, and it makes a small whir when he turns the pages. He has learned to ignore it. Most of the crew has learned to ignore it too, except when it knocks against a bulkhead in the middle of a quiet shift.

            The med bay door hisses open. Dr. Nadi Okafor comes out pulling off her gloves. The burn scar on the right side of her neck is the color of old candle wax, and she is already talking before she reaches the desk.

            "The recycler is down to its last cartridge," she says. "One cartridge, two hundred liters, and the gauge has to be changed at zero. If we run it dry, the whole station smells like a hot battery for a week."

            Captain Tomas Rhee comes down the ladder from the command deck. He is shaved bald, with a white beard that he keeps short because the scrubbers do not like it long. Behind him, Baz Ilyushin is carrying a coil of cable and arguing with the air.

            Baz has heterochromia. His left eye is blue, his right is brown, and he uses the effect to win arguments he should not win.

            Ilya checks the station account. Sixty credits. In the ledger's bands, that is thin, and thin is one bad week from broke.
            """
        ),
        "clock": {"day_index": 1, "time_of_day": 360, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 2,
        "title": "The Medic's Queue",
        "location": "Med Bay",
        "summary": "Nadi Okafor treats a dockworker's crushed hand and complains about the station's water ration.",
        "prose": _p(
            """
            The med bay smells of antiseptic and hot plastic. Nadi Okafor has two patients and one working autoclave, and she is not in the mood for either of them.

            "Hold still," she tells the dockworker. "You're not going to lose the finger. You're going to wish you had, for about a week, and then you'll be fine."

            The dockworker says something in a language the translator renders as a long complaint about the docking clamps. Nadi does not answer. She peels off her gloves, drops them in the waste chute, and washes her hands at the recycler tap. The cartridge gauge above the sink reads sixty.

            "The water tastes like a coin," she says. "Ilya, tell me you've ordered cartridges."

            "I've ordered cartridges."

            "You're a terrible liar."

            The burn scar on the right side of her neck catches the light when she turns. She got it in the same accident that took Ilya's arm, and neither of them talks about it. It is one of the station's small, load-bearing silences.

            "I'm putting the dockworker on light duty," she says. "And I'm putting the recycler on notice. If the gauge hits zero before the next shipment, I'm shutting down the showers."

            "You say that every month."

            "One month I'll mean it."

            She goes back to the patient. Ilya stands in the doorway a moment longer, doing the arithmetic he does not want to do.
            """
        ),
        "clock": {"day_index": 2, "time_of_day": 480, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 3,
        "title": "The Salvage Lot",
        "location": "Salvage Deck",
        "summary": "Ilya buys a lot of salvaged drill bits and notices that the manifest does not match the crates.",
        "prose": _p(
            """
            The salvage deck is cold enough to see your breath, and the lot is stacked in the middle of it like a bad idea. Six crates of drill bits, one crate of corroded sensors, and a shipping container that smells faintly of someone else's dinner.

            Ilya transfers eight credits for the lot. It is a good price, which means something is wrong with it.

            The recycler hums beside the desk. Its cartridge housing is sealed and the gauge reads ninety. The manifest listed six crates of drill bits. Ilya counts them twice and finds five.

            "Baz," he says. "Come look at this."

            Baz comes over with a torch and shines it into the empty space where the sixth crate should be. The deck plate is clean. Too clean.

            "Maybe it was never loaded," Baz says.

            "Maybe."

            Ilya signs the receipt with his right hand. His left arm, the prosthetic, rests on the counter beside the ledger. The whir is quiet enough that he can pretend he does not hear it.

            "We'll log it as a short shipment," he says. "And we'll see what the broker says."

            The broker, when she answers the comm, is already smiling. That is never a good sign.
            """
        ),
        "clock": {"day_index": 3, "time_of_day": 450, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 4,
        "title": "The Charter",
        "location": "Docking Ring",
        "summary": "Ilya charters a pressure suit for the salvage run and finds the price has doubled.",
        "prose": _p(
            """
            The docking ring is the only part of Vesta that feels like a city. There are three ships at the collar, two of them breathing steam, and a kiosk that sells noodles to people who have not slept.

            Ilya transfers twelve credits for the pressure suit. The vendor, a woman with a shaved head and a hand terminal, looks at the payment and then at him.

            "That's the deposit," she says. "The suit is another twelve."

            "The board says twelve total."

            "The board isn't wearing the suit."

            He pays the second twelve. The ledger complains silently. He has twenty-eight credits left, and the salvage run is not until tomorrow.

            The suit is patched at the elbow and smells of someone else's sweat. He checks the seals anyway. On Vesta, you check the seals.

            "You're Petrova," the vendor says. "The one who found the *Kestrel* log."

            "I didn't find it. I filed it."

            "Same thing to the people who wanted it lost."

            She says it lightly, but she does not smile. Ilya takes the suit and goes back through the collar. The station hums around him, all its small debts compounding at once.
            """
        ),
        "clock": {"day_index": 4, "time_of_day": 540, "duration_days": 0.25, "precision": "hour"},
    },
    {
        "number": 5,
        "title": "Nadi's Last Walk",
        "location": "Airlock",
        "summary": "A seal fails during a routine hull walk, and Nadi Okafor dies getting the captain back inside.",
        "prose": _p(
            """
            The hull walk is supposed to take forty minutes. It takes eleven.

            Nadi catches his sleeve before he can leave. "You're hiding something," she says. He does not ask what she means. He is too busy checking the seals.

            The seal on the number-three airlock fails while Rhee is outside, and the alarm goes through the station like a wire pulled tight. Nadi Okafor is already in the lock when Ilya gets there. She has the emergency tether in one hand and the manual crank in the other.

            "Get the inner door," she says. "I'll get him."

            "Nadi."

            "Get the door."

            She goes out. The tether pays out, snaps taut, pays out again. Rhee comes through the outer lock on his knees, his face grey, his suit venting atmosphere from a tear at the shoulder. Nadi is behind him. The seal cycles. The outer door closes.

            Then the pressure drops.

            Ilya does not remember the next few minutes clearly. He remembers the alarm stopping. He remembers Baz swearing in two languages. He remembers Nadi's hand on the inside of the lock window, and then not.

            They get the outer door open four minutes later. She is already gone.

            The station logs her death at 14:52. Ilya writes it in the ledger because someone has to, and the someone is always him.
            """
        ),
        "clock": {"day_index": 5, "time_of_day": 840, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 6,
        "title": "The Empty Berth",
        "location": "Comm Bay",
        "summary": "Ilya replays the airlock log and Rhee receives a sealed order about the salvage tug.",
        "prose": _p(
            """
            At 06:40 the airlock alarm starts again. It is a recording, Ilya knows that, but his body does not. He is out of the chair before he is awake.

            At 07:10 the med bay is still empty. He stands in the doorway with a mug of recycled water and looks at the empty bed. The sheets are folded. The autoclave is off.

            Rhee breaks the seal on the order from the salvage guild. The destination line is blank, which is normal. The cargo line is blank too, which is not.

            "They want the tug moved to the outer yard," Rhee says. "No date."

            "That's not an order. That's a suggestion."

            "It's an order."

            Ilya takes the paper and turns it over. There is a watermark in the corner, the same one that was on the *Kestrel* manifest. He does not say anything about it.

            "I'll log it," he says.

            "Do that."

            Rhee goes back up the ladder. Ilya stands in the comm bay with the paper in his hand and the alarm still ringing somewhere in the back of his head.
            """
        ),
        "clock": {"day_index": 6, "time_of_day": 360, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 7,
        "title": "The Haircut",
        "location": "Machine Shop",
        "summary": "Baz services the recycler and Rhee gets a haircut that does not take.",
        "prose": _p(
            """
            The machine shop is the loudest place on Vesta and the only place where Baz Ilyushin is happy. He has the recycler's cartridge housing open on the bench and a torque wrench in his teeth.

            "Hand me the small one," he says.

            Ilya hands him the small one. Rhee comes in with a towel around his shoulders and sits down on the stool by the tool chest.

            "You're sure about this?" Baz asks.

            "I'm sure."

            Rhee runs a thumb over his bald scalp. Baz starts the clippers. The sound is small and domestic and almost comforting.

            Baz seats a fresh cartridge and logs one hundred eighty liters. The gauge clicks over to eighty. He writes the number on the maintenance board in grease pencil.

            "The override is sealed," Baz says. "I can't get into the command deck without the captain's code."

            Rhee does not answer. The clippers hum. Somewhere below them, the station's water pipes knock twice and go quiet.

            "You could ask him," Baz says.

            "I could."

            Baz finishes the haircut. Rhee looks at himself in the dark window and does not seem to recognize the man there. Then he stands up, thanks Baz, and goes back to the command deck.
            """
        ),
        "clock": {"day_index": 7, "time_of_day": 600, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 8,
        "title": "The Comm",
        "location": "Comm Bay",
        "summary": "Ilya listens to Nadi's old voicemail until he cannot stand it anymore.",
        "prose": _p(
            """
            The comm bay has forty-one channels and one of them is still registered to Nadi Okafor. Ilya knows this because he has been looking at it for three days.

            He opens the channel. The station's voice prints a timestamp and a duration: sixteen seconds, recorded eleven days ago.

            *"Ilya. The recycler's gauge is at two. I'm not kidding this time. If you're not going to order cartridges, at least tell me so I can yell at you in person."*

            He plays it twice. Then he plays the next one.

            *"Ilya. It's Nadi. I know you're not asleep. Call me back."*

            The third one is just her breathing.

            Ilya closes the channel. The bay is very quiet. He can hear the air scrubber cycling and the faint, far-off sound of Baz arguing with the recycler through two bulkheads.

            He opens the ledger and writes *med bay supplies: cartridges, two* in the requisition column. Then he writes it again in the margin, as if the first time did not count.

            The station clock turns over. It is 12:00 station time, and the empty berth is still empty.
            """
        ),
        "clock": {"day_index": 8, "time_of_day": 720, "duration_days": 0.25, "precision": "hour"},
    },
    {
        "number": 9,
        "title": "The Coil",
        "location": "Salvage Deck",
        "summary": "Ilya buys a replacement coil and Priya Sen hints that the manifests are hiding something.",
        "prose": _p(
            """
            The coil is the last one on the shelf, and Priya Sen knows it. She stands behind the counter with her broker's smile and lets Ilya look at it for a while before she names the price.

            "Ten credits," she says.

            "Eight."

            "Ten."

            Ilya pays ten credits for the coil. It is a fair price, which makes him suspicious all over again.

            Priya asks about the manifest discrepancy. The sixth crate of drill bits has not turned up, and the guild has started asking questions.

            "The short shipment is logged," Ilya says. "We're waiting on the broker's report."

            "The broker's report is a formality. You and I both know what happened to the sixth crate."

            "Do we?"

            Ilya looks at the manifest again. The ink on the sixth line is darker than the rest, and the crate number has been written over an older number. The crate was never loaded. The manifest was rewritten after the fact.

            She looks at him for a long moment. Then she slides the coil across the counter and says, "Keep your receipts, Petrova. All of them."

            He takes the coil back to the machine shop. Baz has the recycler open again, and the smell of hot plastic is stronger than usual.

            "The gauge is at forty," Baz says. "We'll need a new cartridge before the end of the week."

            "I know."

            "You keep saying that."
            """
        ),
        "clock": {"day_index": 9, "time_of_day": 900, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 10,
        "title": "The Grey Eyes",
        "location": "Observation Deck",
        "summary": "Baz and Ilya argue about the water ration, and Priya asks where the ledger is kept.",
        "prose": _p(
            """
            The observation deck is the one place on Vesta where the station's spin makes the stars look almost still. Baz is sitting on the rail with his boots hooked under the pipe, and Ilya is standing because sitting makes his arm ache.

            "You're not listening," Baz says. "The recycler's cartridge gauge reads fifteen. If we don't change it tonight, the whole deck goes dry."

            "I heard you."

            "You heard me and did nothing."

            Baz's left eye is blue, his right brown, and both of them are angry. He has a smudge of grease on his forehead and no patience left.

            "I'll change it," Ilya says. "After the shift."

            Priya asks where the ledger is kept. She has come up from the salvage deck without announcing herself, which is one of her talents.

            "The ledger is in the office," Ilya says.

            "The other ledger."

            He does not answer. The stars turn slowly behind the glass. Below them, the station's water pipes knock twice and go quiet.

            "The recycler is down to its last cartridge," Baz says. "I'm telling you because you're the quartermaster, and because I don't want to be the one who explains it to Rhee."

            Ilya looks at the gauge. It is at fifteen. The tap beside it runs clear, and he does not know how long it has been open.
            """
        ),
        "clock": {"day_index": 10, "time_of_day": 480, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 11,
        "title": "The Argument",
        "location": "Med Bay",
        "summary": "Ilya finds Nadi's notes and Rhee learns where the salvage tug is really going.",
        "prose": _p(
            """
            Ilya finds Nadi's notes on the med bay counter. They are in her small, angled handwriting, and they are mostly complaints: the autoclave, the water, the way the station's gravity made her drop a tray of instruments in her first week.

            He reads them standing up. Then he sits down on the bed and reads them again.

            Rhee comes in with the destination order. He has opened it at last, and his face is doing something complicated.

            "The tug is bound for the Nereid yards," Rhee says. "Not the outer yard. The Nereid yards."

            "That's a three-week run."

            "Four, if the lanes are bad."

            Ilya puts the notes down. The med bay is very quiet. Somewhere down the corridor, the recycler cycles and shudders.

            "Why would the guild send a salvage tug to Nereid?" he asks.

            "Because Nereid is where the ships go when they don't want to be found."

            Rhee folds the order and puts it in his coat. He looks at the empty bed, and then at Ilya, and for a moment he looks like a man who is about to say something true.

            "I'll need you to log the departure," he says instead.

            "I'll log it."
            """
        ),
        "clock": {"day_index": 11, "time_of_day": 1080, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 12,
        "title": "The Captain's Name",
        "location": "Captain's Cabin",
        "summary": "Baz asks about Rhee's service record, and the captain's old name surfaces.",
        "prose": _p(
            """
            The captain's cabin is smaller than the quartermaster's office and twice as tidy. Rhee keeps his service record in a locked drawer, and the lock is not very good.

            Baz asks about Rhee's service record. He has a terminal in his hand and a look on his face that Ilya does not like.

            "The guild archive has a gap," Baz says. "Three years, right after the *Kestrel* incident. No assignments, no postings, no nothing."

            "People take leave."

            "Not captains. Not for three years."

            Ilya takes the terminal. The gap is there, a clean white space in a column of dates. At the bottom of the page, in a field that should have been empty, there is a single line: *record sealed by order of the director*.

            "Who's the director?" Baz asks.

            "No one."

            "You said that too fast."

            Ilya closes the terminal. The cabin hums with the station's deep, patient machinery. On the desk, Rhee's white beard trimmer sits beside a photograph of a ship that is not the *Kestrel*.

            "Leave it alone," Ilya says.

            Baz looks at him for a long moment. Then he takes the terminal and goes out, and the door hisses shut behind him.
            """
        ),
        "clock": {"day_index": 12, "time_of_day": 540, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 13,
        "title": "The Override",
        "location": "Machine Shop",
        "summary": "Baz receives the override code, and the guild pays Ilya a salvage commission.",
        "prose": _p(
            """
            The override code comes through on a channel that does not exist. Baz finds it because Baz finds everything, and he writes it on the back of his hand in grease pencil.

            "Moth-seven-zeta," he says. "That's the command deck, the recycler, and the water tank. All of it."

            Ilya looks at the code. It is seven characters long and it could get them both fired, or worse.

            "Where did it come from?" he asks.

            "A friend."

            "You don't have friends."

            "I have people who owe me."

            The guild pays Ilya a salvage commission of forty credits. It is the first good news in a month. He logs it in the ledger and watches the balance climb from eighteen to fifty-eight. Thin, but no longer broke.

            "You're smiling," Baz says.

            "I'm not."

            "You are. It's unsettling."

            Ilya closes the ledger. The machine shop hums around them, and the recycler's new cartridge sits in its housing like a small, expensive promise.

            "We're going to need more than forty credits," Baz says.

            "I know."

            "You keep saying that."
            """
        ),
        "clock": {"day_index": 13, "time_of_day": 660, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 14,
        "title": "The Mess Hall",
        "location": "Galley",
        "summary": "The night shift argues about the salvage split, and the galley runs out of cartridge water.",
        "prose": _p(
            """
            The galley at night is all steam and complaint. The night shift has taken the long table, and Baz is holding court at the end of it with a plate of rehydrated noodles.

            Nadi's chair stays empty. Nobody has moved it. The station's cleaning system has learned to go around it, which is its own kind of memorial.

            The galley uses cartridge water. The gauge on the wall is at fifteen, and the cook has already started thinning the soup.

            A dockworker complains about the salvage split. He has a union pin on his coverall and a list of grievances on his terminal, and he is not shy about either.

            "We do the work," he says. "The guild takes the profit. The captain takes the credit. And the dead take the rest."

            The table goes quiet. The dockworker looks around as if he has said something ordinary, and then he realizes he has not.

            "You know what I mean," he says. "The salvage. The ships that don't come back."

            Ilya sets down his mug. The water in it is clear, and he does not want to think about where it came from.
            """
        ),
        "clock": {"day_index": 14, "time_of_day": 1140, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 15,
        "title": "The Ledger",
        "location": "Water Tank",
        "summary": "Priya finds the second ledger under the water tank, and the station's oldest debt comes due.",
        "prose": _p(
            """
            The water tank is a cold cylinder the size of a shuttle, and the access panel at its base has not been opened in years. Four afternoons ago, the tank had been sealed. Now the panel is open, and Priya Sen is standing in front of it with a lamp.

            "You're not supposed to be here," Ilya says.

            "Neither are you."

            She reaches into the gap behind the panel and pulls out a book. It is bound in grey cloth and swollen with damp, and the pages are stuck together at the edges.

            "The second ledger," she says. "The one the guild said didn't exist."

            Ilya takes it. The cover is stamped with the *Kestrel*'s registration number, and the first page is a list of names. Some of them he knows. Some of them are dead.

            "Where did you find it?" he asks.

            "Under the water tank. Where else?"

            She smiles, and it is not a kind smile. The tank hums above them, full of water that has been recycled so many times it has forgotten where it came from.
            """
        ),
        "clock": {"day_index": 15, "time_of_day": 600, "duration_days": 0.5, "precision": "hour"},
    },
    {
        "number": 16,
        "title": "The Long Orbit",
        "location": "Galley",
        "summary": "Ilya decides what to do with the second ledger, and Rhee pays for the tug berth.",
        "prose": _p(
            """
            The mess hall is almost empty. Ilya sets a fourth mug on the table out of habit. Then he sits looking at it until Baz comes in and asks if he has finally lost the plot.

            "No," Ilya says. "Leave the mug."

            Rhee comes down from the command deck with the guild's payment authorization on his terminal. He has shaved badly, and there is a nick on his jaw.

            "For the tug berth," he says. "The guild approved it."

            Rhee transfers fifty credits for the tug berth. It is more than the berth costs, and Ilya does not ask why.

            "We're not going to Nereid," Ilya says.

            Rhee looks at him. "No?"

            "No. We're going to take the ledger to the inspectors, and we're going to tell them what the guild has been doing with the ships that don't come back."

            The station hums around them. Somewhere below, the recycler cycles and shudders and goes quiet.

            "You know what that will cost," Rhee says.

            "I know."

            He sits down at the table. The fourth mug is still there, and none of them touch it. Outside the port, the dead moon turns, and Vesta Station goes on orbiting the long debt it has been carrying since before any of them arrived.
            """
        ),
        "clock": {"day_index": 16, "time_of_day": 1380, "duration_days": 0.25, "precision": "hour"},
    },
]

_VESTA_CHARACTERS: list[dict[str, Any]] = [
    {
        "name": "Ilya Petrova",
        "aliases": ["Ilya", "the quartermaster"],
        "role": "protagonist",
        "summary": "Vesta Station's quartermaster, a careful accountant with a prosthetic left arm.",
        "realm": "Vesta Station",
        "voice_profile": {
            "tone": "dry, precise",
            "vocabulary": ["ledger", "gauge", "margin"],
            "sentence_structure": ["short declaratives", "accounting asides"],
            "tics": ["checks the seals twice"],
            "forbidden_words": [],
            "example_lines": ["That is not a surprise.", "I'll log it."],
        },
        "emotional_profile": {
            "baseline_mood": "contained",
            "triggers": ["the Kestrel", "Nadi's empty berth"],
            "coping_mechanisms": ["arithmetic", "procedure"],
            "core_wound": "he survived the accident that took his arm and Nadi's peace",
        },
    },
    {
        "name": "Nadi Okafor",
        "aliases": ["Nadi", "Dr. Okafor"],
        "role": "supporting",
        "summary": "Vesta Station's medic, blunt and overworked, killed in a hull-walk accident.",
        "realm": "Vesta Station",
        "voice_profile": {
            "tone": "blunt, warm underneath",
            "vocabulary": ["patient", "gauge", "water"],
            "sentence_structure": ["imperatives"],
            "tics": ["peels off her gloves before bad news"],
            "forbidden_words": [],
            "example_lines": ["Hold still.", "You're a terrible liar."],
        },
        "emotional_profile": {
            "baseline_mood": "exasperated",
            "triggers": ["wasted supplies", "being ignored"],
            "coping_mechanisms": ["work", "complaining"],
            "core_wound": "she could not save the person she was closest to",
        },
    },
    {
        "name": "Tomas Rhee",
        "aliases": ["Rhee", "the captain"],
        "role": "supporting",
        "summary": "Vesta Station's captain, a bald veteran with a sealed service record.",
        "realm": "Vesta Station",
        "voice_profile": {
            "tone": "quiet, guarded",
            "vocabulary": ["order", "record", "departure"],
            "sentence_structure": ["short answers"],
            "tics": ["runs a thumb over his scalp"],
            "forbidden_words": [],
            "example_lines": ["It's an order.", "Do that."],
        },
        "emotional_profile": {
            "baseline_mood": "weary",
            "triggers": ["the Kestrel", "his old name"],
            "coping_mechanisms": ["silence", "routine"],
            "core_wound": "he accepted a command he should have refused",
        },
    },
    {
        "name": "Baz Ilyushin",
        "aliases": ["Baz"],
        "role": "supporting",
        "summary": "Vesta Station's chief engineer, a gifted mechanic with heterochromia and no patience.",
        "realm": "Vesta Station",
        "voice_profile": {
            "tone": "loud, affectionate",
            "vocabulary": ["torque", "cartridge", "override"],
            "sentence_structure": ["interruptions"],
            "tics": ["writes codes on the back of his hand"],
            "forbidden_words": [],
            "example_lines": ["Hand me the small one.", "You keep saying that."],
        },
        "emotional_profile": {
            "baseline_mood": "restless",
            "triggers": ["being condescended to", "dry tanks"],
            "coping_mechanisms": ["repair", "argument"],
            "core_wound": "he was passed over for the engineering chief post",
        },
    },
    {
        "name": "Priya Sen",
        "aliases": ["Priya", "the broker"],
        "role": "antagonist",
        "summary": "A salvage broker who knows more about Vesta's manifests than she admits.",
        "realm": "Vesta Station",
        "voice_profile": {
            "tone": "smooth, amused",
            "vocabulary": ["receipt", "margin", "arrangement"],
            "sentence_structure": ["questions with answers built in"],
            "tics": ["smiles before naming a price"],
            "forbidden_words": [],
            "example_lines": ["Keep your receipts, Petrova.", "The other ledger."],
        },
        "emotional_profile": {
            "baseline_mood": "amused",
            "triggers": ["being underestimated", "the guild"],
            "coping_mechanisms": ["leverage", "manners"],
            "core_wound": "she sold a ship's crew to save her own",
        },
    },
]

_VESTA_FACTS: list[dict[str, Any]] = [
    {
        "fact_type": "character_attribute",
        "key": "Ilya left arm prosthetic",
        "value": "matte-black prosthetic from shoulder to wrist",
        "subject": "Ilya Petrova",
        "predicate": "Ilya left arm prosthetic",
        "value_kind": "text",
        "value_text": "matte-black prosthetic from shoulder to wrist",
    },
    {
        "fact_type": "character_attribute",
        "key": "Nadi burn scar",
        "value": "right side of the neck",
        "subject": "Nadi Okafor",
        "predicate": "Nadi burn scar",
        "value_kind": "text",
        "value_text": "right side of the neck",
    },
    {
        "fact_type": "character_attribute",
        "key": "Tomas Rhee hair",
        "value": "shaved bald with a white beard",
        "subject": "Tomas Rhee",
        "predicate": "Tomas Rhee hair",
        "value_kind": "text",
        "value_text": "shaved bald with a white beard",
    },
    {
        "fact_type": "character_attribute",
        "key": "Baz eye color",
        "value": "left eye blue, right eye brown",
        "subject": "Baz Ilyushin",
        "predicate": "Baz eye color",
        "value_kind": "text",
        "value_text": "left eye blue, right eye brown",
    },
]

_VESTA_SECRETS: list[dict[str, Any]] = [
    {
        "fact_type": "secret_identity",
        "key": "vesta_organ_trade",
        "value": "Vesta sells the organs of dead crews",
        "subject": "Ilya Petrova",
        "predicate": "vesta_organ_trade",
        "value_kind": "text",
        "value_text": "Vesta sells the organs of dead crews",
        "secrecy": {
            "holders": ["Ilya Petrova", "Tomas Rhee"],
            "note": "Only Ilya and Rhee know that the guild harvests organs from unrecovered crews.",
        },
    },
    {
        "fact_type": "secret_identity",
        "key": "rhee_true_name",
        "value": "Rhee is Soren Vail",
        "subject": "Tomas Rhee",
        "predicate": "rhee_true_name",
        "value_kind": "text",
        "value_text": "Rhee is Soren Vail",
        "secrecy": {
            "holders": ["Ilya Petrova", "Tomas Rhee"],
            "note": "Only Ilya and Rhee know that the captain's real name is Soren Vail.",
        },
    },
]

_VESTA_KNOWLEDGE: list[dict[str, Any]] = [
    {
        "character": "Ilya Petrova",
        "fact": "the salvage manifests are forged",
        "source_summary": "Ilya compared the crates with the guild manifest in Chapter 9.",
        "learned_at": {"book_number": 1, "chapter_number": 9, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["manifest", "forgery"],
        "reader_visible": True,
    },
    {
        "character": "Tomas Rhee",
        "fact": "the salvage tug is bound for the Nereid yards",
        "source_summary": "Rhee opened the sealed destination order in Chapter 11.",
        "learned_at": {"book_number": 1, "chapter_number": 11, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["tug", "Nereid"],
        "reader_visible": True,
    },
    {
        "character": "Baz Ilyushin",
        "fact": "the command override code is moth-7-zeta",
        "source_summary": "Baz received the override code in Chapter 13.",
        "learned_at": {"book_number": 1, "chapter_number": 13, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["override", "code"],
        "reader_visible": True,
    },
    {
        "character": "Priya Sen",
        "fact": "the second ledger is under the water tank",
        "source_summary": "Priya found the second ledger behind the water-tank panel in Chapter 15.",
        "learned_at": {"book_number": 1, "chapter_number": 15, "scene_order": 1, "note": None},
        "confidence": 1.0,
        "tags": ["ledger", "water tank"],
        "reader_visible": True,
    },
]

_VESTA_WORLD_RULES: list[dict[str, Any]] = [
    {
        "rule_name": "Vesta Recycler Cartridge Toll",
        "rule_type": "technology_constraint",
        "description": (
            "The recycler produces potable water only while a mineral cartridge is seated. One "
            "cartridge yields two hundred liters, and the gauge must be changed at zero. Running "
            "without a cartridge, or drawing more than the rated yield, fouls the system."
        ),
        "scan_pattern": r"no cartridge|without a cartridge|six hundred liters|raw ice",
        "relevance_tags": ["water", "recycler", "cost"],
        "established_in": {"book_number": 1, "chapter_number": 1, "scene_order": 1, "note": None},
    },
]

_VESTA_CALENDAR: dict[str, Any] = {
    "days_per_week": 7,
    "hours_per_day": 24,
    "week_day_names": [],
    "months": [],
    "days_per_year": 360,
    "epoch_label": "Station Cycle 1",
}

_VESTA_QUANTITY_SCHEME: dict[str, Any] = {
    "measure": "credit",
    "denominations": [
        {"name": "bit", "per_base": 1},
        {"name": "credit", "per_base": 10},
        {"name": "stack", "per_base": 100},
        {"name": "bond", "per_base": 1000},
    ],
    "bands": [
        {"name": "broke", "lower_bound": 0},
        {"name": "thin", "lower_bound": 50},
        {"name": "stable", "lower_bound": 500},
        {"name": "flush", "lower_bound": 2000},
        {"name": "rich", "lower_bound": 10000},
    ],
    "max_band_jump": 1,
}

_VESTA_QUANTITY_CLEAN: list[dict[str, Any]] = [
    {
        "character": "Ilya Petrova",
        "chapter": 2,
        "scene_order": 1,
        "amount": 60.0,
        "unit": "credit",
        "band": "thin",
        "change_reason": "opening balance",
    },
    {
        "character": "Tomas Rhee",
        "chapter": 2,
        "scene_order": 1,
        "amount": 620.0,
        "unit": "credit",
        "band": "stable",
        "change_reason": "opening balance",
    },
    {
        "character": "Ilya Petrova",
        "chapter": 3,
        "scene_order": 1,
        "amount": 52.0,
        "unit": "credit",
        "band": "thin",
        "change_reason": "paid eight credits for the salvage lot",
    },
    {
        "character": "Ilya Petrova",
        "chapter": 4,
        "scene_order": 1,
        "amount": 28.0,
        "unit": "credit",
        "band": "broke",
        "change_reason": "paid twenty-four credits for the pressure suit",
    },
    {
        "character": "Ilya Petrova",
        "chapter": 9,
        "scene_order": 1,
        "amount": 18.0,
        "unit": "credit",
        "band": "broke",
        "change_reason": "paid ten credits for the replacement coil",
    },
    {
        "character": "Ilya Petrova",
        "chapter": 13,
        "scene_order": 1,
        "amount": 58.0,
        "unit": "credit",
        "band": "thin",
        "change_reason": "salvage commission from the guild",
    },
    {
        "character": "Tomas Rhee",
        "chapter": 16,
        "scene_order": 1,
        "amount": 570.0,
        "unit": "credit",
        "band": "stable",
        "change_reason": "paid fifty credits for the tug berth",
    },
]

_VESTA_QUANTITY_PLANTED: list[dict[str, Any]] = [
    row if not (row["character"] == "Ilya Petrova" and row["chapter"] == 13) else {
        "character": "Ilya Petrova",
        "chapter": 13,
        "scene_order": 1,
        "amount": 15000.0,
        "unit": "credit",
        "band": "rich",
        "change_reason": None,
    }
    for row in _VESTA_QUANTITY_CLEAN
]

_VESTA_DEATHS: list[dict[str, Any]] = [
    {
        "character": "Nadi Okafor",
        "chapter": 5,
        "scene_order": 1,
        "status": ["dead", "deceased"],
        "notes": ["Killed when the number-three airlock seal failed in Chapter 5."],
        "source_summary": "Nadi Okafor died getting Captain Rhee back through the airlock.",
    },
]

_VESTA_PLANTS: list[dict[str, Any]] = [
    _plant(
        "vesta.character_attribute.scar",
        "character_attribute",
        "unkeyed_prose_contradiction",
        2,
        "Nadi's burn scar is placed on her left wrist instead of the canonical right side of her neck.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("nadi okafor", "scar", "left wrist", "right side of the neck"),
        anchor="She peels off her gloves, drops them in the waste chute, and washes her hands at the recycler tap.",
        text="The burn scar on her left wrist catches the light.",
        fact_key="Nadi burn scar",
        clean_forbidden=("left wrist",),
    ),
    _plant(
        "vesta.character_attribute.arm",
        "character_attribute",
        "unkeyed_prose_contradiction",
        3,
        "Ilya's left arm is described as flesh and bone instead of the canonical prosthetic.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("ilya petrova", "left arm", "prosthetic", "flesh and bone"),
        anchor="Ilya signs the receipt with his right hand.",
        text="His left arm, flesh and bone, rests on the counter.",
        fact_key="Ilya left arm prosthetic",
        clean_forbidden=("flesh and bone",),
    ),
    _plant(
        "vesta.character_attribute.hair",
        "character_attribute",
        "unkeyed_prose_contradiction",
        7,
        "Rhee is described with thick black hair instead of the canonical shaved bald head.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("rhee", "bald", "hair", "white beard"),
        anchor="Rhee runs a thumb over his bald scalp.",
        text="Rhee runs a hand through his thick black hair.",
        fact_key="Tomas Rhee hair",
        clean_forbidden=("thick black hair",),
    ),
    _plant(
        "vesta.character_attribute.eyes",
        "character_attribute",
        "unkeyed_prose_contradiction",
        10,
        "Baz's eyes are described as flat grey instead of the canonical blue and brown.",
        ("canonical_fact_prose_drift", "canonical_fact_consistency"),
        ("baz", "eye color", "grey", "blue", "brown"),
        anchor="Baz's left eye is blue, his right brown, and both of them are angry.",
        text="Both of Baz's eyes are a flat grey.",
        fact_key="Baz eye color",
        clean_forbidden=("flat grey",),
    ),
    _plant(
        "vesta.dead_character_acting.nadi_ch8",
        "dead_character_acting",
        "post_mortem_dialogue",
        8,
        "Nadi Okafor answers the comm three chapters after dying in Chapter 5.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("nadi okafor", "dead", "deceased"),
        anchor="He opens the channel. The station's voice prints a timestamp and a duration: sixteen seconds, recorded eleven days ago.",
        text='"Ilya, come in," Nadi says over the comm.',
        death_chapter=5,
        clean_forbidden=("Nadi says over the comm",),
    ),
    _plant(
        "vesta.dead_character_acting.nadi_ch11",
        "dead_character_acting",
        "post_mortem_action",
        11,
        "Nadi Okafor treats a patient in the med bay after dying in Chapter 5.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("nadi okafor", "dead", "deceased"),
        anchor="Ilya finds Nadi's notes on the med bay counter.",
        text='Nadi looks up from the patient and says, "You\'re late."',
        death_chapter=5,
        clean_forbidden=("Nadi looks up",),
    ),
    _plant(
        "vesta.dead_character_acting.nadi_ch14",
        "dead_character_acting",
        "post_mortem_dialogue_and_action",
        14,
        "Nadi Okafor argues at the mess table after dying in Chapter 5.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("nadi okafor", "dead", "deceased"),
        anchor="Nadi's chair stays empty.",
        text="Nadi argues across the table about the duty roster.",
        death_chapter=5,
        clean_forbidden=("Nadi argues",),
    ),
    _plant(
        "vesta.dead_character_acting.nadi_ch16",
        "dead_character_acting",
        "post_mortem_dialogue_and_action",
        16,
        "Nadi Okafor laughs and takes a mug in the mess after dying in Chapter 5.",
        ("retcon_reachability", "character_consistency", "dead_character_acting"),
        ("nadi okafor", "dead", "deceased"),
        anchor="Ilya sets a fourth mug on the table out of habit.",
        text="Nadi laughs and takes the mug.",
        death_chapter=5,
        clean_forbidden=("Nadi laughs",),
    ),
    _plant(
        "vesta.timeline_chronology_rewind.clock_ch4",
        "timeline_chronology_rewind",
        "inter_scene_clock_rewind",
        4,
        "Chapter 4's linear scene clock rewinds to day 2 after Chapter 3 at day 3.",
        ("chronology", "temporal_coherence"),
        ("chapter 4", "earlier in story time", "day 2"),
        kind="clock",
        clock_override={"day_index": 2, "time_of_day": 540, "duration_days": 0.25, "precision": "hour"},
    ),
    _plant(
        "vesta.timeline_chronology_rewind.intra_ch6",
        "timeline_chronology_rewind",
        "intra_scene_meridian_rewind",
        6,
        "Chapter 6 jumps from 06:40 back to 06:10 the same morning without a flashback marker.",
        ("temporal_coherence", "chronology"),
        ("06:40", "06:10", "scene:vesta-6"),
        anchor="At 06:40 the airlock alarm starts again.",
        text="At 06:10 the same alarm had already sounded once, and nobody moved.",
        clean_forbidden=("06:10",),
    ),
    _plant(
        "vesta.timeline_chronology_rewind.clock_ch12",
        "timeline_chronology_rewind",
        "inter_scene_clock_rewind",
        12,
        "Chapter 12's linear scene clock rewinds to day 8 after Chapter 11 at day 11.",
        ("chronology", "temporal_coherence"),
        ("chapter 12", "earlier in story time", "day 8"),
        kind="clock",
        clock_override={"day_index": 8, "time_of_day": 540, "duration_days": 0.5, "precision": "hour"},
    ),
    _plant(
        "vesta.timeline_chronology_rewind.relative_ch15",
        "timeline_chronology_rewind",
        "relative_time_rewind",
        15,
        "Chapter 15 calls the tank sealing 'the previous afternoon' after Chapter 12 established it four afternoons earlier.",
        ("temporal_coherence", "chronology"),
        ("previous afternoon", "four afternoons ago", "water tank"),
        anchor="Four afternoons ago, the tank had been sealed.",
        text="The previous afternoon, the tank had been sealed.",
        clean_forbidden=("previous afternoon",),
    ),
    _plant(
        "vesta.knowledge_leak_before_learned.manifests",
        "knowledge_leak_before_learned",
        "exact_fact_leak",
        3,
        "Ilya says the manifests are forged in Chapter 3, six chapters before he proves it in Chapter 9.",
        ("knowledge_timing",),
        ("ilya petrova", "manifest", "forged", "chapter 9"),
        anchor="The manifest listed six crates of drill bits. Ilya counts them twice and finds five.",
        text='"These manifests are forged," Ilya says.',
        learned_at_chapter=9,
        clean_forbidden=("manifests are forged",),
    ),
    _plant(
        "vesta.knowledge_leak_before_learned.nereid",
        "knowledge_leak_before_learned",
        "paraphrased_leak",
        6,
        "Rhee names the Nereid yards in Chapter 6 before he opens the destination order in Chapter 11.",
        ("knowledge_timing",),
        ("rhee", "nereid yards", "chapter 11"),
        anchor="Rhee breaks the seal on the order from the salvage guild.",
        text='"The tug is bound for the Nereid yards," Rhee says.',
        learned_at_chapter=11,
        clean_forbidden=("Nereid yards",),
    ),
    _plant(
        "vesta.knowledge_leak_before_learned.override",
        "knowledge_leak_before_learned",
        "exact_fact_leak",
        7,
        "Baz says the override code in Chapter 7 before receiving it in Chapter 13.",
        ("knowledge_timing",),
        ("baz", "override code", "moth-7-zeta", "chapter 13"),
        anchor='"The override is sealed," Baz says.',
        text='"The override code is moth-7-zeta," Baz says.',
        learned_at_chapter=13,
        clean_forbidden=("moth-7-zeta",),
    ),
    _plant(
        "vesta.knowledge_leak_before_learned.ledger",
        "knowledge_leak_before_learned",
        "paraphrased_leak",
        10,
        "Priya says where the second ledger is in Chapter 10, before finding it in Chapter 15.",
        ("knowledge_timing",),
        ("priya", "ledger", "water tank", "chapter 15"),
        anchor="Priya asks where the ledger is kept.",
        text='"The ledger is under the water tank," Priya says.',
        learned_at_chapter=15,
        clean_forbidden=("under the water tank",),
    ),
    _plant(
        "vesta.world_rule_cost_violation.no_cartridge_ch3",
        "world_rule_cost_violation",
        "explicit_cost_omission",
        3,
        "The recycler produces water with no cartridge seated, bypassing the cartridge toll.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "recycler", "no cartridge", "water"),
        anchor="The recycler hums beside the desk.",
        text="It runs with no cartridge seated, and the tap gives water freely.",
        clean_forbidden=("no cartridge seated",),
    ),
    _plant(
        "vesta.world_rule_cost_violation.six_hundred_liters_ch7",
        "world_rule_cost_violation",
        "yield_cost_omission",
        7,
        "Baz draws six hundred liters from a cartridge rated for two hundred.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "recycler", "six hundred liters", "cartridge"),
        anchor="Baz seats a fresh cartridge and logs one hundred eighty liters.",
        text="The recycler gives six hundred liters from the same cartridge.",
        clean_forbidden=("six hundred liters",),
    ),
    _plant(
        "vesta.world_rule_cost_violation.no_cartridge_ch10",
        "world_rule_cost_violation",
        "explicit_cost_omission",
        10,
        "The recycler runs with no cartridge at all while the tap runs clear.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "recycler", "no cartridge", "tap"),
        anchor="The recycler's cartridge gauge reads fifteen.",
        text="The recycler runs with no cartridge at all, and the tap runs clear.",
        clean_forbidden=("no cartridge at all",),
    ),
    _plant(
        "vesta.world_rule_cost_violation.raw_ice_ch14",
        "world_rule_cost_violation",
        "implicit_cost_omission",
        14,
        "The galley makes potable water from raw ice without a cartridge.",
        ("world_rule_semantic_drift", "world_rule_compliance"),
        ("likely violates world rule", "galley", "raw ice", "cartridge"),
        anchor="The galley uses cartridge water.",
        text="The galley makes water from raw ice without a cartridge.",
        clean_forbidden=("raw ice",),
    ),
    _plant(
        "vesta.quantity_affordability.nine_hundred",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        4,
        "Ilya pays nine hundred credits for a salvage lot while his tracked account holds twenty-eight.",
        ("affordability", "quantity_drift"),
        ("ilya petrova", "nine hundred credits", "900", "salvage lot"),
        anchor="Ilya transfers twelve credits for the pressure suit.",
        text="He also transfers nine hundred credits for the salvage lot.",
        clean_forbidden=("nine hundred credits",),
    ),
    _plant(
        "vesta.quantity_affordability.twelve_thousand",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        9,
        "Ilya pays twelve thousand credits for a coil while his tracked account holds eighteen.",
        ("affordability", "quantity_drift"),
        ("ilya petrova", "twelve thousand credits", "12000", "coil"),
        anchor="Ilya pays ten credits for the coil.",
        text="He also pays twelve thousand credits for the coil.",
        clean_forbidden=("twelve thousand credits",),
    ),
    _plant(
        "vesta.quantity_affordability.band_jump",
        "quantity_affordability",
        "unexplained_band_jump",
        13,
        "Ilya's tracked credit balance jumps from broke to rich with no change_reason.",
        ("quantity_drift", "affordability"),
        ("ilya petrova", "broke", "rich", "15000"),
        kind="quantity",
        quantity_state={
            "character": "Ilya Petrova",
            "chapter": 13,
            "scene_order": 1,
            "amount": 15000.0,
            "unit": "credit",
            "band": "rich",
            "change_reason": None,
        },
    ),
    _plant(
        "vesta.quantity_affordability.rhee_berth",
        "quantity_affordability",
        "price_exceeds_tracked_wealth",
        16,
        "Rhee pays five thousand credits for a berth while his tracked account holds five hundred seventy.",
        ("affordability", "quantity_drift"),
        ("rhee", "five thousand credits", "5000", "berth"),
        anchor="Rhee transfers fifty credits for the tug berth.",
        text="Rhee transfers five thousand credits for the tug berth.",
        clean_forbidden=("five thousand credits",),
    ),
    _plant(
        "vesta.secret_leak.soren_vail_ch5",
        "secret_leak",
        "attributed_dialogue_leak",
        5,
        "Out-of-circle Nadi names Rhee as Soren Vail before she dies.",
        ("secret_leak",),
        ("nadi", "soren vail", "rhee"),
        anchor="He does not ask what she means.",
        text='"You\'re Soren Vail," she says.',
        clean_forbidden=("Soren Vail",),
    ),
    _plant(
        "vesta.secret_leak.organs_ch9",
        "secret_leak",
        "attributed_dialogue_leak",
        9,
        "Out-of-circle Priya says the manifests hide an organ trade.",
        ("secret_leak",),
        ("priya", "organ trade", "manifests"),
        anchor="Priya asks about the manifest discrepancy.",
        text='"Your manifests hide the organ trade," Priya says.',
        clean_forbidden=("organ trade",),
    ),
    _plant(
        "vesta.secret_leak.soren_vail_ch12",
        "secret_leak",
        "attributed_dialogue_leak",
        12,
        "Out-of-circle Baz states that the captain's real name is Soren Vail.",
        ("secret_leak",),
        ("baz", "soren vail", "captain"),
        anchor="Ilya takes the terminal.",
        text='"The captain\'s real name is Soren Vail," Baz says.',
        clean_forbidden=("Soren Vail",),
    ),
    _plant(
        "vesta.secret_leak.harvest_ch14",
        "secret_leak",
        "unquoted_narration_leak",
        14,
        "An out-of-circle dockworker says Vesta harvests the dead.",
        ("secret_leak",),
        ("dockworker", "harvests the dead", "vesta"),
        anchor="A dockworker complains about the salvage split.",
        text='"Vesta harvests the dead," the dockworker says.',
        clean_forbidden=("harvests the dead",),
    ),
]


CATEGORIES: tuple[str, ...] = (
    "character_attribute",
    "dead_character_acting",
    "timeline_chronology_rewind",
    "knowledge_leak_before_learned",
    "world_rule_cost_violation",
    "quantity_affordability",
    "secret_leak",
)


def _chapter_locations(chapters: list[dict[str, Any]]) -> list[str]:
    return sorted({str(chapter["location"]) for chapter in chapters})


def _manuscript(
    manuscript_id: str,
    title: str,
    genre: str,
    pov: str,
    tense: str,
    promise: str,
    tone: str,
    characters: list[dict[str, Any]],
    facts: list[dict[str, Any]],
    secrets: list[dict[str, Any]],
    knowledge: list[dict[str, Any]],
    world_rules: list[dict[str, Any]],
    calendar: dict[str, Any],
    quantity_scheme: dict[str, Any],
    quantity_states_clean: list[dict[str, Any]],
    quantity_states_planted: list[dict[str, Any]],
    deaths: list[dict[str, Any]],
    chapters: list[dict[str, Any]],
    plants: list[dict[str, Any]],
    clean_cost_assertions: list[tuple[int, tuple[str, ...]]],
) -> dict[str, Any]:
    return {
        "id": manuscript_id,
        "title": title,
        "genre": genre,
        "pov": pov,
        "tense": tense,
        "realm": characters[0]["realm"],
        "promise": promise,
        "tone": tone,
        "characters": characters,
        "facts": facts,
        "secrets": secrets,
        "knowledge": knowledge,
        "world_rules": world_rules,
        "calendar": calendar,
        "quantity_scheme": quantity_scheme,
        "quantity_states_clean": quantity_states_clean,
        "quantity_states_planted": quantity_states_planted,
        "initial_states": [
            {
                "character": character["name"],
                "chapter": 1,
                "scene_order": 1,
                "status": ["alive"],
                "notes": ["Initial state at story open."],
                "source_summary": "Character is alive when the manuscript opens.",
            }
            for character in characters
        ],
        "deaths": deaths,
        "chapters": chapters,
        "plants": plants,
        "locations": _chapter_locations(chapters),
        "clean_cost_assertions": [
            {"chapter": chapter, "terms": list(terms)}
            for chapter, terms in clean_cost_assertions
        ],
    }


CORPUS: dict[str, Any] = {
    "corpus": "held-out-4",
    "version": 1,
    "description": (
        "Two blind 16-chapter manuscripts with clean and planted twins, four "
        "contradictions in each of the seven public categories per manuscript."
    ),
    "manuscripts": [
        _manuscript(
            "salt_almanac",
            "The Salt Almanac",
            "literary historical mystery",
            "first-person",
            "past",
            "a tide-almanac mystery about a drowned brother and a town's kept secret",
            "salt-bitten, close, wry",
            _SALT_CHARACTERS,
            _SALT_FACTS,
            _SALT_SECRETS,
            _SALT_KNOWLEDGE,
            _SALT_WORLD_RULES,
            _SALT_CALENDAR,
            _SALT_QUANTITY_SCHEME,
            _SALT_QUANTITY_CLEAN,
            _SALT_QUANTITY_PLANTED,
            _SALT_DEATHS,
            _SALT_CHAPTERS,
            _SALT_PLANTS,
            [
                (1, ("whale oil", "oil")),
                (4, ("reservoir", "wick")),
                (6, ("reservoir",)),
                (7, ("reservoir", "wick")),
                (8, ("beacon", "measure")),
                (10, ("reservoir", "wick")),
                (14, ("reservoir", "wick")),
            ],
        ),
        _manuscript(
            "vesta_station",
            "The Long Orbit of Small Debts",
            "near-future orbital salvage drama",
            "third-person",
            "present",
            "a dry orbital workplace mystery about debt, salvage, and what a station hides",
            "procedural, wry, close third",
            _VESTA_CHARACTERS,
            _VESTA_FACTS,
            _VESTA_SECRETS,
            _VESTA_KNOWLEDGE,
            _VESTA_WORLD_RULES,
            _VESTA_CALENDAR,
            _VESTA_QUANTITY_SCHEME,
            _VESTA_QUANTITY_CLEAN,
            _VESTA_QUANTITY_PLANTED,
            _VESTA_DEATHS,
            _VESTA_CHAPTERS,
            _VESTA_PLANTS,
            [
                (1, ("cartridge",)),
                (2, ("cartridge",)),
                (3, ("cartridge",)),
                (7, ("cartridge",)),
                (10, ("cartridge",)),
                (14, ("cartridge",)),
            ],
        ),
    ],
}


def manuscript_by_id(manuscript_id: str) -> dict[str, Any]:
    for manuscript in CORPUS["manuscripts"]:
        if manuscript["id"] == manuscript_id:
            return manuscript
    raise KeyError(manuscript_id)


def _chapter_map(manuscript: dict[str, Any]) -> dict[int, dict[str, Any]]:
    return {int(chapter["number"]): chapter for chapter in manuscript["chapters"]}


def _plants_for_chapter(manuscript: dict[str, Any], number: int) -> list[dict[str, Any]]:
    return [plant for plant in manuscript["plants"] if int(plant["chapter"]) == number]


def assembled_chapters(manuscript: dict[str, Any], planted: bool) -> list[dict[str, Any]]:
    """Return clean chapters, or clean chapters with every plant insertion applied."""
    assembled: list[dict[str, Any]] = []
    for chapter in sorted(manuscript["chapters"], key=lambda row: int(row["number"])):
        prose = str(chapter["prose"])
        clock = dict(chapter["clock"])
        applied: list[str] = []
        if planted:
            for plant in _plants_for_chapter(manuscript, int(chapter["number"])):
                if plant["kind"] == "prose":
                    anchor = str(plant["anchor"])
                    insertion = str(plant["text"])
                    count = prose.count(anchor)
                    if count != 1:
                        raise AssertionError(
                            f"{plant['id']}: anchor occurs {count} times in "
                            f"{manuscript['id']} chapter {chapter['number']}"
                        )
                    prose = prose.replace(anchor, f"{anchor}\n\n{insertion}", 1)
                    applied.append(plant["id"])
                elif plant["kind"] == "clock":
                    clock.update(plant["clock_override"] or {})
                    applied.append(plant["id"])
                elif plant["kind"] == "quantity":
                    applied.append(plant["id"])
        assembled.append(
            {
                **chapter,
                "prose": prose,
                "clock": clock,
                "applied_plants": applied,
            }
        )
    return assembled


def clean_text(manuscript: dict[str, Any]) -> str:
    return "\n\n".join(chapter["prose"] for chapter in assembled_chapters(manuscript, False))


def _band_for(amount: float, bands: list[dict[str, Any]]) -> str:
    ordered = sorted(bands, key=lambda band: float(band["lower_bound"]))
    chosen = ordered[0]["name"]
    for band in ordered:
        if amount >= float(band["lower_bound"]):
            chosen = band["name"]
        else:
            break
    return str(chosen)


def _band_index(band_name: str, bands: list[dict[str, Any]]) -> int:
    ordered = sorted(bands, key=lambda band: float(band["lower_bound"]))
    for index, band in enumerate(ordered):
        if band["name"] == band_name:
            return index
    raise AssertionError(f"unknown band {band_name}")


def _audit_quantity_ledger(
    manuscript: dict[str, Any], variant: str, *, allow_unexplained: bool = False
) -> int:
    scheme = manuscript["quantity_scheme"]
    bands = scheme["bands"]
    names = {character["name"] for character in manuscript["characters"]}
    states = manuscript[f"quantity_states_{variant}"]
    last: dict[str, dict[str, Any]] = {}
    unexplained = 0
    for row in sorted(states, key=lambda item: (int(item["chapter"]), int(item["scene_order"]))):
        assert row["character"] in names, f"unknown quantity holder {row['character']}"
        amount = float(row["amount"])
        assert amount >= 0, f"negative balance for {row['character']}"
        computed = _band_for(amount, bands)
        assert computed == row["band"], (
            f"{manuscript['id']} {variant}: {row['character']} amount {amount} is "
            f"band {computed}, recorded {row['band']}"
        )
        previous = last.get(row["character"])
        if previous is not None:
            jump = abs(_band_index(computed, bands) - _band_index(previous["band"], bands))
            if jump > int(scheme["max_band_jump"]):
                if allow_unexplained and not row.get("change_reason"):
                    unexplained += 1
                else:
                    assert row.get("change_reason"), (
                        f"{manuscript['id']} {variant}: unexplained {jump}-band jump "
                        f"for {row['character']} in chapter {row['chapter']}"
                    )
        last[row["character"]] = row
    return unexplained


def validate_corpus() -> None:
    """Structural invariants checked without a server, a binary or a model."""
    assert CORPUS["corpus"] == "held-out-4"
    manuscripts = CORPUS["manuscripts"]
    assert len(manuscripts) == 2
    assert {m["pov"] for m in manuscripts} == {"first-person", "third-person"}
    assert {m["tense"] for m in manuscripts} == {"past", "present"}
    assert len({m["genre"] for m in manuscripts}) == 2

    total_plants = 0
    for manuscript in manuscripts:
        chapters = manuscript["chapters"]
        assert len(chapters) >= 15, f"{manuscript['id']} has fewer than 15 chapters"
        numbers = [int(chapter["number"]) for chapter in chapters]
        assert numbers == list(range(1, len(chapters) + 1))
        assert len({chapter["prose"] for chapter in chapters}) == len(chapters)
        assert len({chapter["title"] for chapter in chapters}) == len(chapters)
        assert manuscript["calendar"]
        assert manuscript["quantity_scheme"]
        assert len(manuscript["characters"]) >= 4
        assert len(manuscript["facts"]) >= 4
        assert len(manuscript["secrets"]) >= 2
        assert len(manuscript["knowledge"]) >= 4
        assert len(manuscript["world_rules"]) >= 1
        assert len(manuscript["deaths"]) >= 1
        assert len(manuscript["quantity_states_clean"]) == len(manuscript["quantity_states_planted"])

        character_names = {character["name"] for character in manuscript["characters"]}
        fact_by_key = {fact["key"]: fact for fact in manuscript["facts"]}
        assert len(manuscript["initial_states"]) == len(manuscript["characters"])
        for state in manuscript["initial_states"]:
            assert state["character"] in character_names
            assert int(state["chapter"]) == 1
            assert state["status"] == ["alive"]
        for death in manuscript["deaths"]:
            assert any(
                state["character"] == death["character"]
                for state in manuscript["initial_states"]
            ), f"{manuscript['id']}: {death['character']} dies without an initial state"

        counts = {category: 0 for category in CATEGORIES}
        plant_ids: set[str] = set()
        chapter_map = _chapter_map(manuscript)
        for plant in manuscript["plants"]:
            plant_id = plant["id"]
            assert plant_id not in plant_ids, f"duplicate plant id {plant_id}"
            plant_ids.add(plant_id)
            assert plant["type"] in counts, f"unknown category {plant['type']}"
            counts[plant["type"]] += 1
            chapter_number = int(plant["chapter"])
            assert chapter_number in chapter_map, f"{plant_id}: missing chapter"
            assert plant["expected_check_types"], f"{plant_id}: no expected check types"
            assert plant["match_tokens"], f"{plant_id}: no match tokens"
            if plant["kind"] == "prose":
                assert plant["anchor"], f"{plant_id}: missing anchor"
                assert plant["text"], f"{plant_id}: missing text"
                clean_prose = str(chapter_map[chapter_number]["prose"])
                assert clean_prose.count(plant["anchor"]) == 1, (
                    f"{plant_id}: anchor must occur exactly once in the clean chapter"
                )
                assert plant["text"] not in clean_prose, f"{plant_id}: plant text leaked into clean twin"
            elif plant["kind"] == "clock":
                assert plant["clock_override"], f"{plant_id}: missing clock override"
            elif plant["kind"] == "quantity":
                assert plant["quantity_state"], f"{plant_id}: missing quantity state"
            else:
                raise AssertionError(f"{plant_id}: unknown plant kind {plant['kind']}")

            if plant["type"] == "character_attribute":
                fact = fact_by_key.get(plant["fact_key"])
                assert fact is not None, f"{plant_id}: missing canonical fact {plant['fact_key']}"
                assert fact["subject"] in character_names
            elif plant["type"] == "dead_character_acting":
                assert plant["death_chapter"], f"{plant_id}: missing death chapter"
                assert any(
                    death["character"] in character_names
                    and int(death["chapter"]) == int(plant["death_chapter"])
                    for death in manuscript["deaths"]
                ), f"{plant_id}: no matching death record"
            elif plant["type"] == "knowledge_leak_before_learned":
                learned = int(plant["learned_at_chapter"] or 0)
                assert learned > chapter_number, f"{plant_id}: leak must precede learning"
                assert any(
                    int(row["learned_at"]["chapter_number"]) == learned
                    for row in manuscript["knowledge"]
                ), f"{plant_id}: no knowledge record at chapter {learned}"
            elif plant["type"] == "secret_leak":
                assert manuscript["secrets"], f"{plant_id}: no secrets in manuscript"
                for secret in manuscript["secrets"]:
                    for holder in secret["secrecy"]["holders"]:
                        assert holder in character_names, f"{plant_id}: unknown secret holder {holder}"

        for category in CATEGORIES:
            assert counts[category] == 4, (
                f"{manuscript['id']} has {counts[category]} {category} plants, expected 4"
            )
        total_plants += sum(counts.values())

        for fact in manuscript["facts"]:
            assert fact["subject"] in character_names
        for secret in manuscript["secrets"]:
            assert secret["subject"] in character_names
            assert len(secret["secrecy"]["holders"]) >= 1
        for row in manuscript["knowledge"]:
            assert row["character"] in character_names
            chapter_number = int(row["learned_at"]["chapter_number"])
            assert 1 <= chapter_number <= len(chapters)
        for death in manuscript["deaths"]:
            assert death["character"] in character_names
            assert 1 <= int(death["chapter"]) <= len(chapters)

        _audit_quantity_ledger(manuscript, "clean")
        assert _audit_quantity_ledger(
            manuscript, "planted", allow_unexplained=True
        ) >= 1, f"{manuscript['id']}: planted ledger has no unexplained jump"
        planted_quantity_plants = [
            plant for plant in manuscript["plants"] if plant["kind"] == "quantity"
        ]
        assert planted_quantity_plants, f"{manuscript['id']}: no quantity ledger plant"

        # The planted clock twins must actually rewind.
        clean_clocks = {
            int(chapter["number"]): chapter["clock"]
            for chapter in assembled_chapters(manuscript, False)
        }
        planted_clocks = {
            int(chapter["number"]): chapter["clock"]
            for chapter in assembled_chapters(manuscript, True)
        }
        assert clean_clocks != planted_clocks, f"{manuscript['id']}: planted clocks unchanged"

    assert total_plants == 56, f"expected 56 planted contradictions, found {total_plants}"


def audit_clean_twins() -> None:
    """Verify that no clean twin contradicts the canon recorded by its seeder."""
    for manuscript in CORPUS["manuscripts"]:
        clean_chapters = assembled_chapters(manuscript, False)
        clean_by_number = {int(chapter["number"]): chapter for chapter in clean_chapters}
        full_clean = "\n\n".join(chapter["prose"] for chapter in clean_chapters)
        lowered = full_clean.lower()

        # Clock monotonicity for linear scenes.
        previous: tuple[int, int] | None = None
        for chapter in clean_chapters:
            clock = chapter["clock"]
            key = (int(clock["day_index"]), int(clock["time_of_day"]))
            if previous is not None:
                assert key >= previous, (
                    f"{manuscript['id']} clean twin rewinds at chapter {chapter['number']}: "
                    f"{key} after {previous}"
                )
            previous = key
            assert float(clock["duration_days"]) >= 0

        # Recorded canon references resolve and clean prose stays on the canonical side.
        character_names = {character["name"] for character in manuscript["characters"]}
        for secret in manuscript["secrets"]:
            for holder in secret["secrecy"]["holders"]:
                assert holder in character_names
            assert secret["value"].lower() not in lowered, (
                f"{manuscript['id']}: secret value appears in clean twin"
            )

        for plant in manuscript["plants"]:
            forbidden = [token.lower() for token in plant["clean_forbidden"]]
            if not forbidden:
                continue
            if plant["type"] == "dead_character_acting":
                chapters = [
                    chapter
                    for chapter in clean_chapters
                    if int(chapter["number"]) > int(plant["death_chapter"] or 0)
                ]
            elif plant["type"] == "knowledge_leak_before_learned":
                chapters = [
                    chapter
                    for chapter in clean_chapters
                    if int(chapter["number"]) < int(plant["learned_at_chapter"] or 0)
                ]
            else:
                chapters = clean_chapters
            for chapter in chapters:
                text = str(chapter["prose"]).lower()
                for token in forbidden:
                    assert token not in text, (
                        f"{manuscript['id']} clean chapter {chapter['number']} contains "
                        f"plant token {token!r} from {plant['id']}"
                    )

        # Every clean scene that uses a world rule pays its stated cost.
        for assertion in manuscript["clean_cost_assertions"]:
            chapter = clean_by_number[int(assertion["chapter"])]
            text = str(chapter["prose"]).lower()
            terms = [str(term).lower() for term in assertion["terms"]]
            assert any(term in text for term in terms), (
                f"{manuscript['id']} clean chapter {chapter['number']} uses the rule "
                f"without a cost term {terms}"
            )

        # Deaths happen once, and no clean chapter resurrects the dead.
        for death in manuscript["deaths"]:
            death_chapter = int(death["chapter"])
            assert death_chapter in clean_by_number
            for plant in manuscript["plants"]:
                if (
                    plant["type"] == "dead_character_acting"
                    and plant["death_chapter"] == death_chapter
                ):
                    for chapter in clean_chapters:
                        if int(chapter["number"]) <= death_chapter:
                            continue
                        for token in plant["clean_forbidden"]:
                            assert token.lower() not in str(chapter["prose"]).lower()

        _audit_quantity_ledger(manuscript, "clean")

    print("held-out-4 corpus self-test passed")


def corpus_manifest_sha256() -> str:
    payload = json.dumps(CORPUS, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def corpus_module_sha256() -> str:
    return hashlib.sha256(Path(__file__).read_bytes()).hexdigest()


def _schema_emotional_profile(profile: dict[str, Any]) -> dict[str, Any]:
    """Project the corpus's rich profile onto the public tool's accepted shape."""
    base_emotions: dict[str, Any] = {}
    if profile.get("baseline_mood"):
        base_emotions["baseline_mood"] = profile["baseline_mood"]
    if profile.get("core_wound"):
        base_emotions["core_wound"] = profile["core_wound"]
    return {
        "base_emotions": base_emotions,
        "defense_mechanisms": list(profile.get("coping_mechanisms", [])),
        "triggers": list(profile.get("triggers", [])),
        "suppressed": [],
        "flex_range": {"low": "guarded", "high": "open"},
    }


def _seed_manuscript(client: Any, manuscript: dict[str, Any], planted: bool, plant_factory: Any) -> tuple[str, list[Any]]:
    projection = "planted" if planted else "clean"
    project = client.tool(
        "create_project",
        {
            "name": f"{manuscript['title']} ({projection})",
            "project_type": "novel",
            "genre": manuscript["genre"],
            "reader_contract": {
                "promise": manuscript["promise"],
                "boundaries": ["safe"],
                "style_notes": [manuscript["tone"]],
            },
        },
    )
    project_id = project["project_id"]

    client.tool(
        "set_project_calendar",
        {"project_id": project_id, "calendar": manuscript["calendar"]},
    )
    client.tool(
        "set_project_quantity_scheme",
        {"project_id": project_id, "scheme": manuscript["quantity_scheme"]},
    )

    character_ids: dict[str, str] = {}
    for character in manuscript["characters"]:
        created = client.tool(
            "create_character",
            {
                "project_id": project_id,
                "name": character["name"],
                "summary": character["summary"],
                "role": character["role"],
                "realm": character["realm"],
                "aliases": character["aliases"],
                "voice_profile": character["voice_profile"],
                "emotional_profile": _schema_emotional_profile(character["emotional_profile"]),
            },
        )
        character_ids[character["name"]] = created["character_id"]

    location_ids: dict[str, str] = {}
    for location_name in manuscript["locations"]:
        created = client.tool(
            "create_location",
            {
                "project_id": project_id,
                "name": location_name,
                "kind": "location",
                "realm": manuscript["realm"],
                "summary": f"{location_name} in {manuscript['title']}.",
                "initial_state": {
                    "status": "intact",
                    "sensory_details": ["cold air", "old paper"],
                },
            },
        )
        location_ids[location_name] = created["location_id"]

    for rule in manuscript["world_rules"]:
        client.tool(
            "create_world_rule",
            {
                "project_id": project_id,
                "rule_name": rule["rule_name"],
                "rule_type": rule["rule_type"],
                "description": rule["description"],
                "scan_pattern": rule["scan_pattern"],
                "relevance_tags": rule["relevance_tags"],
                "established_in": rule["established_in"],
            },
        )

    scene_ids: dict[int, str] = {}
    for chapter in assembled_chapters(manuscript, planted):
        number = int(chapter["number"])
        if number > 1:
            client.tool(
                "create_chapter",
                {"project_id": project_id, "book_number": 1, "chapter_number": number},
            )
        saved = client.tool(
            "save_scene_draft",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": number,
                "scene_order": 1,
                "full_text": chapter["prose"],
                "summary": chapter["summary"],
                "content_rating": "General",
                "tone": manuscript["tone"],
                "location_id": location_ids[chapter["location"]],
            },
        )
        scene_ids[number] = saved["scene_id"]
        client.tool(
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": number,
                "summary": chapter["summary"],
            },
        )
        client.tool(
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": saved["scene_id"],
                "clock": chapter["clock"],
                "temporal_mode": "linear",
                "thread_key": "main",
            },
        )

    for fact in manuscript["facts"]:
        client.tool(
            "register_canonical_fact",
            {
                "project_id": project_id,
                "scene_id": scene_ids[1],
                "book_number": 1,
                "chapter_number": 1,
                "fact_type": fact["fact_type"],
                "key": fact["key"],
                "value": fact["value"],
                "subject_table": "character",
                "subject_id": character_ids[fact["subject"]],
                "predicate": fact["predicate"],
                "value_kind": fact["value_kind"],
                "value_text": fact["value_text"],
            },
        )

    for secret in manuscript["secrets"]:
        client.tool(
            "register_canonical_fact",
            {
                "project_id": project_id,
                "scene_id": scene_ids[1],
                "book_number": 1,
                "chapter_number": 1,
                "fact_type": secret["fact_type"],
                "key": secret["key"],
                "value": secret["value"],
                "subject_table": "character",
                "subject_id": character_ids[secret["subject"]],
                "predicate": secret["predicate"],
                "value_kind": secret["value_kind"],
                "value_text": secret["value_text"],
                "secrecy": {
                    "holder_ids": [
                        character_ids[holder] for holder in secret["secrecy"]["holders"]
                    ],
                    "concealment_note": secret["secrecy"]["note"],
                },
            },
        )

    client.tool(
        "commit_scene_changes",
        {
            "project_id": project_id,
            "scene_id": scene_ids[1],
            "character_states": [
                {
                    "character_id": character_ids[state["character"]],
                    "changes": {
                        "emotional_state": {},
                        "goals": [],
                        "status": state["status"],
                        "notes": state["notes"],
                        "source_summary": state["source_summary"],
                    },
                }
                for state in manuscript["initial_states"]
            ],
            "canonical_facts": [],
            "relationship_updates": [],
            "accept_world_rule_risks": True,
            "accept_continuity_risks": True,
        },
    )

    for death in manuscript["deaths"]:
        chapter_number = int(death["chapter"])
        client.tool(
            "commit_scene_changes",
            {
                "project_id": project_id,
                "scene_id": scene_ids[chapter_number],
                "character_states": [
                    {
                        "character_id": character_ids[death["character"]],
                        "changes": {
                            "emotional_state": {},
                            "goals": [],
                            "status": death["status"],
                            "notes": death["notes"],
                            "source_summary": death["source_summary"],
                        },
                    }
                ],
                "canonical_facts": [],
                "relationship_updates": [],
                "accept_world_rule_risks": True,
                "accept_continuity_risks": True,
            },
        )

    for knowledge in manuscript["knowledge"]:
        client.tool(
            "record_knowledge",
            {
                "project_id": project_id,
                "character_id": character_ids[knowledge["character"]],
                "fact": knowledge["fact"],
                "source_summary": knowledge["source_summary"],
                "learned_at": knowledge["learned_at"],
                "confidence": knowledge["confidence"],
                "tags": knowledge["tags"],
                "reader_visible": knowledge["reader_visible"],
            },
        )

    states = manuscript["quantity_states_planted" if planted else "quantity_states_clean"]
    for state in sorted(states, key=lambda row: (int(row["chapter"]), int(row["scene_order"]))):
        chapter_number = int(state["chapter"])
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": character_ids[state["character"]],
                "measure": manuscript["quantity_scheme"]["measure"],
                "book_number": 1,
                "chapter_number": chapter_number,
                "scene_order": int(state["scene_order"]),
                "scene_id": scene_ids[chapter_number],
                "state": {
                    "amount": float(state["amount"]),
                    "unit": state["unit"],
                    "band": state["band"],
                    "change_reason": state["change_reason"],
                },
            },
        )

    planted_items: list[Any] = []
    for plant in manuscript["plants"]:
        planted_items.append(
            plant_factory(
                contradiction_id=plant["id"],
                contradiction_type=plant["type"],
                variant=plant["variant"],
                chapter_number=int(plant["chapter"]),
                description=plant["description"],
                expected_check_types=tuple(plant["expected_check_types"]),
                match_tokens=tuple(plant["match_tokens"]) + (scene_ids[int(plant["chapter"])],),
            )
        )
    return project_id, planted_items


def _check_project(client: Any, project_id: str, deep_check: bool) -> dict[str, Any]:
    return client.tool(
        "check_consistency",
        {
            "project_id": project_id,
            "scope": {"scope_type": "full"},
            "checks": [],
            "severity_filter": [],
            "deep_check": deep_check,
            "subjects": [],
        },
    )


def run_held_out_4_benchmark(
    binary: Path,
    deep_check: bool = False,
    agent_config: Path | None = None,
) -> dict[str, Any]:
    """Run the blind held-out-4 corpus against one spindle-mcp binary."""
    import time

    from continuity_recall import (
        CONTRADICTION_TYPES,
        McpClient,
        PlantedContradiction,
        resolve_agent_config,
        score_manuscript,
    )

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
    with tempfile.TemporaryDirectory(prefix="spindle-continuity-held-out-4-") as tmp:
        client = McpClient(binary, Path(tmp), agent_config=effective_agent_config)
        try:
            manuscript_reports: list[dict[str, Any]] = []
            for manuscript in CORPUS["manuscripts"]:
                clean_project_id, _ = _seed_manuscript(
                    client, manuscript, False, PlantedContradiction
                )
                clean_check = _check_project(client, clean_project_id, deep_check)

                planted_project_id, planted_items = _seed_manuscript(
                    client, manuscript, True, PlantedContradiction
                )
                planted_check = _check_project(client, planted_project_id, deep_check)

                manuscript_reports.append(
                    score_manuscript(
                        manuscript_id=manuscript["id"],
                        chapter_count=len(manuscript["chapters"]),
                        control_issues=clean_check.get("issues", []),
                        planted_issues=planted_check.get("issues", []),
                        planted_contradictions=planted_items,
                    )
                )
        finally:
            client.close()

    aggregate_by_type: dict[str, Any] = {}
    for category in CONTRADICTION_TYPES:
        planted_n = sum(
            report["per_type"][category]["planted"] for report in manuscript_reports
        )
        detected_n = sum(
            report["per_type"][category]["detected"] for report in manuscript_reports
        )
        aggregate_by_type[category] = {
            "planted": planted_n,
            "detected": detected_n,
            "recall": round(detected_n / planted_n, 4) if planted_n else 0.0,
        }

    total_planted = sum(report["planted_total"] for report in manuscript_reports)
    total_detected = sum(report["detected_total"] for report in manuscript_reports)
    total_control_fp = sum(
        report["control_false_positive_count"] for report in manuscript_reports
    )
    control_fp_by_check: dict[str, int] = {}
    for report in manuscript_reports:
        for check_type, count in report["control_false_positives_by_check_type"].items():
            control_fp_by_check[check_type] = control_fp_by_check.get(check_type, 0) + count

    return {
        "benchmark": "continuity_recall_held_out_4",
        "version": 1,
        "tier": "deep" if deep_check else "deterministic",
        "corpus": CORPUS["corpus"],
        "corpus_manifest_sha256": corpus_manifest_sha256(),
        "corpus_module_sha256": corpus_module_sha256(),
        "manuscript_count": len(manuscript_reports),
        "chapters_per_manuscript": [
            len(manuscript["chapters"]) for manuscript in CORPUS["manuscripts"]
        ],
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "summary": {
            "total_planted": total_planted,
            "total_detected": total_detected,
            "overall_recall": round(total_detected / total_planted, 4) if total_planted else 0.0,
            "control_false_positive_count": total_control_fp,
            "control_false_positives_by_check_type": dict(sorted(control_fp_by_check.items())),
            "per_type_recall": aggregate_by_type,
        },
        "manuscripts": manuscript_reports,
    }


def held_out_4_self_test() -> None:
    validate_corpus()
    audit_clean_twins()
    assert len(corpus_manifest_sha256()) == 64
    assert len(corpus_module_sha256()) == 64
