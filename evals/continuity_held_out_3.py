#!/usr/bin/env python3
"""Blind held-out-3 continuity corpus.

Two independently authored manuscripts (one first-person present-tense gothic
mystery, one third-person past-tense desert fantasy) with 28 planted
contradictions each, four in every public contradiction category, plus clean
twin controls.  The plants are ordinary novelist revisions: a substituted
attribute, a dead character who speaks again, an elapsed-time phrase that
disagrees with the recorded clock, a secret spoken by someone outside its
circle.  They were written without inspecting detector implementations and are
deliberately varied in phrasing and surface form.

Scoring reuses the shared `score_manuscript` matcher.  In addition to the
shared localized recall, this corpus reports a weaker *type-present* channel:
an issue whose expected check type appears at the plant's own scene even when
the finding's message does not carry one of the plant's content tokens.
Control warnings and errors are reported separately from all-severity control
false positives so an advisory cannot be confused with a contradiction.

The corpus, its plants, and its expectations are frozen once the before run is
recorded; only genuine harness bugs may change them afterwards.
"""

from __future__ import annotations

from typing import Any

from continuity_recall import CONTRADICTION_TYPES, PlantedContradiction, score_manuscript


# ---------------------------------------------------------------------------
# Manuscript 1: The Salt Orchard (first person, present tense, gothic mystery)
# ---------------------------------------------------------------------------

SALT_ORCHARD: dict[str, Any] = {
    "manuscript_id": "salt_orchard",
    "title": "The Salt Orchard",
    "genre": "gothic mystery",
    "pov": "first person present",
    "project_type": "novel",
    "reader_contract": {
        "promise": "A marine botanist returns to her drowned island and finds that every kindness there has a price.",
        "style_notes": [
            "First person present tense, close and sensory.",
            "Salt, rust, and domestic detail carry the horror.",
            "The supernatural is reported as ordinary weather.",
        ],
        "boundaries": ["No on-page sexual content", "No harm to children"],
    },
    "chapters": [
        {
            "number": 1,
            "title": "The Ferry",
            "day_index": 1,
            "time_of_day": 1060,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "quay",
            "summary": "Iris Calloway returns to Wrenhythe and watches Nell ring the tide bell.",
            "prose": """The ferry smells of diesel and wet wool. Tobin Sarrow stands at the wheel with his blind left eye turned to the weather and his good eye on the channel. He has carried me to Wrenhythe every summer of my life and never once asked why I stopped coming.

"You picked a bad week," he says. "The orchard's flowering early."

I write my name in the manifest with my left hand, the way I have signed everything since I was six. My grey eyes water in the wind, and the burn scar on my left forearm aches under my cuff the way it always does before rain.

Wrenhythe comes up out of the mist: a hundred slate roofs, the lighthouse, the drowned orchard running down into the salt marsh. Nell Vane waits on the quay with the tide bell. At dusk she tears a strip from her apron, wraps it around the clapper, and pulls. The note goes out over the water like a nail drawn from a board. Nell sits down on the bench and forgets her own name for a moment. Then she remembers it and laughs.

"Welcome home, Iris," she says. "Your sister's up at the light.\"""",
        },
        {
            "number": 2,
            "title": "The Empty Light",
            "day_index": 2,
            "time_of_day": 540,
            "duration_days": 0.2,
            "pov": "iris",
            "location": "lighthouse",
            "summary": "Iris finds the lighthouse empty and a rusted iron key in the tea tin.",
            "prose": """Maren is not at the lighthouse. Her coat hangs on its hook, stiff with salt, and the kettle is still warm. The lamp room is cold. A note on the table says: Gone to the north shoal. Back by dark. -M.

I climb the stairs anyway. From the gallery I can see the whole island: the chapel roof, the ferry slip, the orchard rows running out into the salt water like something drowned and still growing.

In the photograph on the sill, Maren is nineteen, her green eyes squinted against the sun, the tip of her right ring finger missing where the winch took it. She is laughing at whoever holds the camera. I was twelve. I remember the joke, and then I do not.

In the tea tin I find an iron key, furred with rust, and a scrap of paper with six numbers on it. The numbers mean nothing to me. I put the key in my pocket. Outside, the tide is coming in over the orchard grass, and the trees stand in it without seeming to mind.""",
        },
        {
            "number": 3,
            "title": "Aunt Fen",
            "day_index": 4,
            "time_of_day": 1140,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "house",
            "summary": "Aunt Fen dies, and Iris rings the tide bell and pays its price.",
            "prose": """Aunt Fen dies in her chair on the third night, with the radio on and her tea gone cold. Dr. Pike signs the certificate at the kitchen table. "She was eighty-seven," he says. "Her heart simply stopped." Nell cries without making any noise. I make the tea because someone has to.

We ring the bell for her at dusk. I tear a strip from my shirt tail, wrap the clapper, and pull until the rope burns my palm. The bell takes something. I lose the kitchen at the old house - the smell of bread, my mother's hands in the dough. I know it was there. I can feel the shape of the missing thing, the way you feel a tooth.

"It always takes a little," Nell says. "That's the price."

We bury Fen on the hill above the orchard. The soil is thin and full of shells. I keep thinking I should say something, but the bell has already taken the words I would have used.""",
        },
        {
            "number": 4,
            "title": "The Ferry Ledger",
            "day_index": 6,
            "time_of_day": 600,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "quay",
            "summary": "Iris pays Tobin for the crossing.",
            "prose": """Tobin keeps the ferry ledger in an oilcloth wallet behind the ticket window. "Nine silver," he says, "same as always." I count nine pennies into his palm. My purse holds nine silver and change; now it holds change. I write the receipt with my left hand and he watches me do it.

"You still write like your father," he says.

The wind shifts. Out on the water the orchard is a dark smear, and beyond it the lighthouse stands with its lamp unlit. Tobin closes the wallet and looks at the sky.

"Your sister was out again last night," he says. "I saw the boat light."

"She's always out."

"Not like that."

I do not ask what he means. I have learned not to ask Tobin Sarrow questions on the sixth day of a month.""",
        },
        {
            "number": 5,
            "title": "The Salt Orchard",
            "day_index": 7,
            "time_of_day": 480,
            "duration_days": 0.3,
            "pov": "iris",
            "location": "orchard",
            "summary": "Iris and Bran survey the salt orchard.",
            "prose": """Bran Hollis meets me at the orchard gate with two pairs of gloves and a billhook. He is forty now, with a diver's flat stare and a laugh that arrives late, as if it has had to swim a long way.

We walk the rows. The trees are old and crooked, grafted onto salt-tolerant stock, and the fruit hangs green and hard. Bran cuts a canker out of one trunk and shows me the ring in the wood.

"Your aunt let this place go," he says. "Your sister wouldn't sell it. Now your aunt's dead, and the Rooks are circling."

"Let them circle," I say.

In the afternoon we find a stone marker under the oldest tree. The carving is worn to a thumbprint. Bran says it marks the first Calloway who planted here, and that the salt has been eating her name for two hundred years.

At the gate he takes my elbow. "If you go out to the wreck," he says, "take me with you.\"""",
        },
        {
            "number": 6,
            "title": "The Lamp Line",
            "day_index": 8,
            "time_of_day": 1260,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "lighthouse",
            "summary": "Maren dies in the storm; Iris notices the lamp line has been cut.",
            "prose": """Maren goes out to the north shoal at dusk. The lighthouse lamp has been failing for a week, and the parts are on the mainland, and she will not wait. I stand on the quay and watch the boat's light shrink.

The storm comes in fast. By midnight the wind is tearing shingles off the boathouse. Nell and I ring the bell, but the bell is for the living, not for the sea.

They find Maren at dawn, tangled in the mooring chain, her right hand still closed around a wrench. Dr. Pike says she was in the water for hours. He says the cold would have taken her before the rocks.

Aunt Fen had been in the ground four days. Now there are two graves on the hill.

I take the iron key out of my pocket and hold it until it warms. The lamp line is still cut. I do not know who cut it. I do not know why I think someone did.""",
        },
        {
            "number": 7,
            "title": "The Wake",
            "day_index": 9,
            "time_of_day": 1080,
            "duration_days": 0.2,
            "pov": "iris",
            "location": "house",
            "summary": "At Maren's wake Silas speaks with Iris, and she tests the iron rule.",
            "prose": """The wake is at the house. Nell puts out bread and cold mutton and a bowl of salt. People come and go. Silas Rook stands by the window in a good grey coat and does not eat.

"Your sister was careless with the lamp," he says. "She was always so sure she could do it alone."

I set down my cup. Tobin Sarrow stands by the door with his blind left eye milky and pale, his good eye on Silas.

Later I take the iron key out to the orchard and bury it a hand deep, painted with lampblack, the way Nell showed me. In the morning it is pitted and furred with rust. The salt has been at it all night.

I put the key back in my pocket and go inside. The house is full of people who loved my sister and no one who knows who cut the lamp line.""",
        },
        {
            "number": 8,
            "title": "The Buyer",
            "day_index": 12,
            "time_of_day": 960,
            "duration_days": 0.2,
            "pov": "iris",
            "location": "house",
            "summary": "Silas Rook makes an offer; Iris lights the house with the last of the oil.",
            "prose": """Silas comes back with a folder and a bottle of good whisky. He wants the orchard. He wants the lighthouse too, though he is careful not to say so first. He does not open the folder. "A fair offer," he says. "You will not get a better one from the trust."

I fill the two lamps I can with the last of the oil and set them on the sill, because the generator is out and I am not going to sit in the dark while Silas Rook talks about money. The flames make the room feel smaller. Silas watches me over the top of his glass.

"You cannot keep it," he says. "The sea is taking the orchard a row at a time. In ten years there will be nothing here but mud."

"Then I will sell mud," I say.

He smiles as if I have said something clever. The lamps burn until midnight. In the morning both are still lit.""",
        },
        {
            "number": 9,
            "title": "Bran's Dive",
            "day_index": 13,
            "time_of_day": 720,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "wreck",
            "summary": "Bran drowns on the wreck.",
            "prose": """Bran dives on the wreck of Maren's boat. He goes down at slack water with a torch and a line, and he comes up twice with nothing but a brass fitting. The third time he does not come up.

We find him under the hull, tangled in the same chain that took Maren. Nell is shouting Bran's name into the water. The doctor comes. The priest comes. The sea does not come for Bran; we have to go and get him. I hold the line while Tobin pulls.

At the house that night I cannot get warm. I sit by the stove with a blanket around my shoulders and my wet boots steaming. The kettle sings. I do not sleep.

In the morning the stove is out and my boots are dry, and I do not remember taking them off.""",
        },
        {
            "number": 10,
            "title": "The Chapel Floor",
            "day_index": 16,
            "time_of_day": 660,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "chapel",
            "summary": "Iris finds the orchard ledger in the chapel.",
            "prose": """Bran had been dead three days, and the chapel still smells of his cigarettes. I row myself out to the chapel rock. The oars knock in the locks.

Nell is waiting on the steps. "The chapel is open," she says. "I lit a candle for Bran."

I go down into the chapel alone. Under the third pew the floorboards are loose. In the space beneath, wrapped in oilcloth, I find the orchard ledger and a bundle of letters tied with fishing line. The top letter is addressed to Seraphine Calloway.

I sit on the chapel floor with the ledger in my lap until the light goes. The ledger says the orchard was never ours to sell. It says the salt has been coming for two hundred years. It says my father knew.""",
        },
        {
            "number": 11,
            "title": "The Third Bell",
            "day_index": 18,
            "time_of_day": 300,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "quay",
            "summary": "Tobin dies; Iris speaks a drowned name after the third bell and fires at the dark.",
            "prose": """Tobin dies on the night tide. They find the ferry drifting with the rope still tied off and no Tobin. I take his shotgun from the boathouse and load the last three shells into the chambers.

At the third bell I stand on the quay and say Maren's name out loud. The bell is still ringing. The salt wind takes my voice for the rest of the night. I can only whisper. I say the name again, and it comes out like a hinge.

Nell comes down the steps with a lamp and asks me to come inside. I do not go. I fire three shells at the dark. The sound goes out over the water and comes back smaller. Then there is nothing but the tide.""",
        },
        {
            "number": 12,
            "title": "The Letter",
            "day_index": 21,
            "time_of_day": 540,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "house",
            "summary": "Silas's letter names forty thousand.",
            "prose": """My voice is still a scrape. Silas's letter arrives on the morning ferry. It is typed on good paper and it offers forty thousand for the orchard, the lighthouse, and the six acres of salt marsh. I read it twice.

I brace my left forearm on the table, and the burn scar shows pale against the wood. The number is more than the trust can match. It is more than the island has ever been worth.

Dr. Pike is at the door before I have finished reading. He asks after my voice and leaves a bottle of something dark. "Do not sell to him," he says. "Not yet."

I fold the letter. Outside, the orchard is white with the first salt bloom.""",
        },
        {
            "number": 13,
            "title": "The Storm",
            "day_index": 24,
            "time_of_day": 420,
            "duration_days": 0.3,
            "pov": "iris",
            "location": "orchard",
            "summary": "A storm floods the orchard and Iris finds a lamp fitting in the oldest tree.",
            "prose": """The storm comes back on the twenty-fourth day. I wake to the bell rope beating against the tower and the orchard under a foot of water. Nell and I sandbag the chapel door until our hands blister. The salt gets into the cuts.

At noon the wind drops. We walk the rows. Three trees have come down, and the oldest one has split to the root. In the split I find a twist of wire and a brass fitting, the same kind Bran brought up from the wreck.

"That's a lamp part," Nell says.

"I know."

I put it in my pocket. The rain starts again before we reach the house. Nell stops at the gate and looks back at the orchard. "Your father used to say the salt was a patient creditor," she says. "It never forgets a debt.\"""",
        },
        {
            "number": 14,
            "title": "The Key",
            "day_index": 27,
            "time_of_day": 600,
            "duration_days": 0.1,
            "pov": "iris",
            "location": "rectory",
            "summary": "Iris pays the registrar to search the deed and learns the Rooks have owned the orchard since before her birth.",
            "prose": """I take the ferry ledger and the chapel key to the rectory. The registrar is a thin woman named Quill who has been waiting for someone to ask. I pay her six silver for the search, and my purse holds nothing after. She counts it twice and writes a receipt.

"The deed is in the Rook name," she says. "It has been since before your father died."

"That is not possible."

She shows me the page. The signature is my father's. The date is three months before I was born.

I walk back along the shore. The orchard is coming into flower, white against the black water. It looks like a wedding and a funeral at once. I have the ledger under my arm and no money in my pocket, and for the first time since the ferry I know exactly what I am going to do.""",
        },
        {
            "number": 15,
            "title": "First Frost",
            "day_index": 30,
            "time_of_day": 480,
            "duration_days": 0.2,
            "pov": "iris",
            "location": "orchard",
            "summary": "Iris waits for the frost, buries Tobin, and lights the lighthouse.",
            "prose": """The first frost is late. I wait. On the thirtieth day it comes, and in the morning I pick an apple from the oldest tree and eat it in the cold. It is sour and good.

Three weeks had passed since Maren drowned. I climb to the lighthouse and light the lamp for the first time since she died. The beam goes out over the water, steady and white. Nobody needs it. I do it anyway.

Tobin's boat is still tied at the quay. Nell wants to sell it. I want to burn it. We compromise and leave it there, knocking against the fenders, until the rope rots through.

At dusk the bell rings. Nell does it without being asked now. The note goes out over the water, and the orchard stands in the tide, and nothing about the island feels solved. It feels inhabited, which is different.""",
        },
        {
            "number": 16,
            "title": "The Crossing",
            "day_index": 33,
            "time_of_day": 960,
            "duration_days": 0.2,
            "pov": "iris",
            "location": "rectory",
            "summary": "The orchard goes to the island trust, and Iris signs the deed.",
            "prose": """The closing is at the rectory on the thirty-third day. I sign the deed with my left hand and the registrar stamps it. Registrar Quill asks whether I want the deed copied. I tell her I do. Nell stands by the window with her hands folded.

The orchard goes to the island trust, not to Silas. It takes three weeks of letters and one threat of a public auction, but the trust has the money and Silas does not have the votes.

Afterward I walk down to the water. The orchard stands in the tide, green and white, and the salt is killing it slowly. That is what orchards do here. They grow for a while and then they drown.

Nell rings the bell at dusk. She tears a strip from her apron. The note goes out over the water. I stand beside her and listen. I have forgotten something, I think. I just cannot remember what.""",
        },
    ],
    "locations": {
        "quay": ("The Ferry Quay", "A slab quay of black stone, slick at low tide."),
        "lighthouse": ("Wrenhythe Light", "A short white tower above the north shoal, its lamp unreliable."),
        "house": ("Calloway House", "A low stone house with a salt-stained garden and a cold kitchen."),
        "orchard": ("The Salt Orchard", "Old apple trees grafted onto salt-tolerant stock, half drowned at high tide."),
        "wreck": ("The North Shoal", "A reef of broken rock where the ferry channel narrows."),
        "chapel": ("Chapel Rock", "A tide-cut chapel on a rock reachable only by boat."),
        "rectory": ("The Rectory", "A whitewashed office that keeps the island's deeds and its silence."),
    },
    "characters": {
        "iris": {
            "name": "Iris Calloway",
            "role": "protagonist",
            "summary": "A marine botanist, forty, left-handed, with grey eyes and a burn scar on her left forearm. She returns to Wrenhythe to settle her aunt's estate.",
            "aliases": ["Iris", "Miss Calloway"],
            "voice_profile": {
                "tone": "dry, exact, tender in spite of itself",
                "example_lines": [
                    '"Then I will sell mud."',
                    '"I have learned not to ask Tobin Sarrow questions on the sixth day of a month."',
                ],
                "vocabulary": ["salt", "tide", "row", "graft"],
                "sentence_structure": ["short declaratives", "occasional list of three concrete things"],
            },
            "emotional_profile": {
                "base_emotions": {"guilt": 0.6, "resolve": 0.5},
                "triggers": ["her sister's voice", "the smell of bread"],
                "defense_mechanisms": ["work", "dry humor"],
            },
            "initial_state": {"status": ["alive"], "goals": ["settle Fen's estate", "learn who cut the lamp line"]},
        },
        "maren": {
            "name": "Maren Calloway",
            "role": "supporting",
            "summary": "Iris's older sister, the lighthouse keeper. Green eyes; the tip of her right ring finger is missing from an old winch accident.",
            "aliases": ["Maren", "the keeper"],
            "voice_profile": {
                "tone": "brisk, amused, unapologetic",
                "example_lines": ['"You look like a drowned cat."', '"I can do it alone."'],
            },
            "emotional_profile": {"base_emotions": {"pride": 0.8, "fatigue": 0.6}, "triggers": ["being helped"]},
            "initial_state": {"status": ["alive"], "goals": ["keep the light burning"]},
        },
        "fen": {
            "name": "Seraphine Fen Calloway",
            "role": "supporting",
            "summary": "Iris's great-aunt, eighty-seven, keeper of the house and its silences. Known to everyone as Aunt Fen.",
            "aliases": ["Aunt Fen", "Fen"],
            "voice_profile": {"tone": "soft, distracted", "example_lines": ['"Is that the ferry?"']},
            "emotional_profile": {"base_emotions": {"nostalgia": 0.7}},
            "initial_state": {"status": ["alive"], "goals": ["keep the house in the family"]},
        },
        "tobin": {
            "name": "Tobin Sarrow",
            "role": "supporting",
            "summary": "The ferryman, sixty-nine, blind in his left eye since a childhood accident. He keeps the island's ferry ledger.",
            "aliases": ["Tobin", "the ferryman"],
            "voice_profile": {"tone": "flat, watchful", "example_lines": ['"Not like that."']},
            "emotional_profile": {"base_emotions": {"wariness": 0.6}},
            "initial_state": {"status": ["alive"], "goals": ["keep the ferry running"]},
        },
        "bran": {
            "name": "Bran Hollis",
            "role": "supporting",
            "summary": "A salvage diver, forty, with a flat stare and a late laugh. He knew Maren better than he admits.",
            "aliases": ["Bran"],
            "voice_profile": {"tone": "slow, careful", "example_lines": ['"Take me with you."']},
            "emotional_profile": {"base_emotions": {"grief": 0.7, "loyalty": 0.8}},
            "initial_state": {"status": ["alive"], "goals": ["recover Maren's boat"]},
        },
        "nell": {
            "name": "Nell Vane",
            "role": "supporting",
            "summary": "The innkeeper and bell-ringer, sixty, practical and kind. She knows the island's rules and most of its secrets.",
            "aliases": ["Nell"],
            "voice_profile": {"tone": "warm, blunt", "example_lines": ['"That is the price."']},
            "emotional_profile": {"base_emotions": {"compassion": 0.8, "fear": 0.3}},
            "initial_state": {"status": ["alive"], "goals": ["keep the island fed"]},
        },
        "silas": {
            "name": "Silas Rook",
            "role": "antagonist",
            "summary": "A land agent in a good grey coat, patient and well funded. He wants the orchard and the lighthouse.",
            "aliases": ["Silas", "Mr. Rook"],
            "voice_profile": {"tone": "reasonable, relentless", "example_lines": ['"You cannot keep it."']},
            "emotional_profile": {"base_emotions": {"patience": 0.9}},
            "initial_state": {"status": ["alive"], "goals": ["acquire the Calloway land"]},
        },
        "pike": {
            "name": "Dr. Ewan Pike",
            "role": "supporting",
            "summary": "The island doctor, fifty-five, who signs certificates and keeps his own counsel.",
            "aliases": ["Dr. Pike", "Pike"],
            "voice_profile": {"tone": "measured", "example_lines": ['"Her heart simply stopped."']},
            "emotional_profile": {"base_emotions": {"caution": 0.7}},
            "initial_state": {"status": ["alive"], "goals": ["keep the island healthy"]},
        },
        "quill": {
            "name": "Registrar Quill",
            "role": "supporting",
            "summary": "The island registrar, thin and exact, keeper of deeds and signatures.",
            "aliases": ["Quill", "the registrar"],
            "voice_profile": {"tone": "clerical", "example_lines": ['"The deed is in the Rook name."']},
            "emotional_profile": {"base_emotions": {"duty": 0.8}},
            "initial_state": {"status": ["alive"], "goals": ["keep the record straight"]},
        },
    },
    "facts": [
        {"key": "iris_handedness", "subject": "iris", "predicate": "handedness", "value_text": "left-handed", "value_kind": "string", "scope": "invariant", "chapter": 1, "aliases": ["writing hand", "signs with"]},
        {"key": "iris_eye_color", "subject": "iris", "predicate": "eye color", "value_text": "grey", "value_kind": "string", "scope": "invariant", "chapter": 1},
        {"key": "iris_burn_scar", "subject": "iris", "predicate": "burn scar", "value_text": "on her left forearm", "value_kind": "string", "scope": "invariant", "chapter": 1, "aliases": ["scar"]},
        {"key": "tobin_blind_eye", "subject": "tobin", "predicate": "left eye", "value_text": "blind and milky", "value_kind": "string", "scope": "invariant", "chapter": 1},
        {"key": "maren_eye_color", "subject": "maren", "predicate": "eye color", "value_text": "green", "value_kind": "string", "scope": "invariant", "chapter": 2},
        {"key": "maren_finger", "subject": "maren", "predicate": "right ring finger", "value_text": "tip missing from an old winch accident", "value_kind": "string", "scope": "invariant", "chapter": 2},
        {"key": "fen_age", "subject": "fen", "predicate": "age", "value_text": "eighty-seven", "value_kind": "string", "scope": "invariant", "chapter": 3},
        {"key": "iris_work", "subject": "iris", "predicate": "profession", "value_text": "marine botanist", "value_kind": "string", "scope": "invariant", "chapter": 1},
    ],
    "deaths": [
        {"character": "fen", "chapter": 3, "source_summary": "Aunt Fen dies in her chair of old age.", "notes": ["Her heart stopped. Dr. Pike signed the certificate."]},
        {"character": "maren", "chapter": 6, "source_summary": "Maren drowns on the north shoal in the storm.", "notes": ["Tangled in the mooring chain."]},
        {"character": "bran", "chapter": 9, "source_summary": "Bran drowns diving on Maren's wreck.", "notes": ["Tangled in the same chain."]},
        {"character": "tobin", "chapter": 11, "source_summary": "Tobin is lost on the night tide; the ferry is found drifting.", "notes": ["No body recovered."]},
    ],
    "knowledge": [
        {"character": "tobin", "fact": "The orchard ledger is under the chapel floor.", "learned_chapter": 6, "source_summary": "Nell tells him after Maren's death."},
        {"character": "bran", "fact": "Maren scuttled her own boat.", "learned_chapter": 8, "source_summary": "He recognizes the cut cable from the wreck."},
        {"character": "pike", "fact": "Silas Rook offered forty thousand for the orchard.", "learned_chapter": 12, "source_summary": "Silas tells him at the rectory."},
        {"character": "nell", "fact": "Seraphine is Fen's real name.", "learned_chapter": 14, "source_summary": "Iris shows her the letter from the chapel."},
    ],
    "secrets": [
        {
            "key": "lamp_line",
            "subject": "maren",
            "predicate": "lamp line",
            "value": "Maren cut the lighthouse lamp line before the rescue boat launched.",
            "holders": ["maren", "nell"],
            "chapter": 1,
            "concealment_note": "Nell denies it with a flat joke and changes the subject.",
        },
        {
            "key": "orchard_deed",
            "subject": "iris",
            "predicate": "orchard deed",
            "value": "Iris's father sold the orchard to the Rook family before she was born.",
            "holders": ["iris", "silas"],
            "chapter": 1,
            "concealment_note": "Iris will not discuss her father's signature with anyone.",
        },
    ],
    "rules": [
        {
            "key": "bell_price",
            "rule_name": "The tide bell's price",
            "rule_type": "magic",
            "description": "The tide bell must be rung at dusk with a strip of cloth torn from the ringer's own clothing. Every ringing takes one memory from the ringer. If the bell is silent, the salt tide rises into the orchard.",
            "scan_pattern": "tide bell",
            "chapter": 1,
        },
        {
            "key": "iron_rule",
            "rule_name": "Iron in the orchard",
            "rule_type": "magic",
            "description": "Iron buried in orchard soil must be painted with lampblack. Unpainted iron is eaten by the salt before dawn.",
            "scan_pattern": "iron",
            "chapter": 2,
        },
        {
            "key": "third_bell",
            "rule_name": "The third bell",
            "rule_type": "magic",
            "description": "No one may speak a drowned name between the third bell and dawn. The sea takes a voice in payment.",
            "scan_pattern": "third bell",
            "chapter": 6,
        },
        {
            "key": "first_frost",
            "rule_name": "First frost",
            "rule_type": "magic",
            "description": "Orchard fruit may be eaten only after the first frost. Fruit picked before the frost causes salt-blindness.",
            "scan_pattern": "first frost",
            "chapter": 2,
        },
    ],
    "quantities": {
        "schemes": [
            {
                "measure": "silver",
                "bands": [
                    {"name": "destitute", "lower_bound": 0},
                    {"name": "comfortable", "lower_bound": 20},
                    {"name": "wealthy", "lower_bound": 100},
                ],
                "denominations": [{"name": "silver", "per_base": 1}, {"name": "penny", "per_base": 1}],
                "max_band_jump": 1,
            },
            {
                "measure": "lamp oil",
                "bands": [
                    {"name": "empty", "lower_bound": 0},
                    {"name": "low", "lower_bound": 1},
                    {"name": "stocked", "lower_bound": 4},
                ],
                "denominations": [{"name": "tin", "per_base": 1}],
                "max_band_jump": 1,
            },
            {
                "measure": "shotgun shells",
                "bands": [
                    {"name": "none", "lower_bound": 0},
                    {"name": "few", "lower_bound": 1},
                    {"name": "plenty", "lower_bound": 8},
                ],
                "denominations": [{"name": "shell", "per_base": 1}],
                "max_band_jump": 1,
            },
        ],
        "states": [
            {"subject": "iris", "measure": "silver", "chapter": 3, "amount": 9, "band": "destitute", "unit": "silver"},
            {"subject": "iris", "measure": "lamp oil", "chapter": 7, "amount": 2, "band": "low", "unit": "tins"},
            {"subject": "tobin", "measure": "shotgun shells", "chapter": 10, "amount": 3, "band": "few", "unit": "shells"},
            {"subject": "iris", "measure": "silver", "chapter": 13, "amount": 6, "band": "destitute", "unit": "silver"},
        ],
    },
    "plants": [
        {
            "plant_id": "SO-ATTR-EYES",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 2,
            "description": "Maren's established green eyes become brown in the lighthouse photograph.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("eye color", "brown"),
            "clean": "her green eyes squinted against the sun",
            "planted": "her brown eyes squinted against the sun",
        },
        {
            "plant_id": "SO-ATTR-TOBIN",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 7,
            "description": "Tobin's blind left eye is described as clear and brown.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("left eye", "brown"),
            "clean": "his blind left eye milky and pale",
            "planted": "his blind left eye clear and brown",
        },
        {
            "plant_id": "SO-ATTR-SCAR",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 12,
            "description": "Iris's burn scar moves from her left forearm to her right.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("burn scar", "right forearm"),
            "clean": "I brace my left forearm on the table, and the burn scar shows pale against the wood.",
            "planted": "I brace my right forearm on the table, and the burn scar shows pale against the wood.",
        },
        {
            "plant_id": "SO-ATTR-HAND",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 16,
            "description": "Left-handed Iris signs the deed with her right hand.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("handedness", "right hand"),
            "clean": "I sign the deed with my left hand",
            "planted": "I sign the deed with my right hand",
        },
        {
            "plant_id": "SO-DEAD-FEN",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_appearance",
            "chapter_number": 7,
            "description": "Aunt Fen, buried in chapter 3, meets Iris at the gate and puts the kettle on.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("fen", "dead"),
            "clean": "Later I take the iron key out to the orchard",
            "planted": "Aunt Fen meets me at the gate with a lantern and asks about the crossing. I tell her it was calm. I tell her Maren is dead. She pats my hand and goes in to put the kettle on.\n\nLater I take the iron key out to the orchard",
        },
        {
            "plant_id": "SO-DEAD-MAREN",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_dialogue",
            "chapter_number": 9,
            "description": "Maren, drowned in chapter 6, pours tea and speaks to Iris.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("maren", "dead"),
            "clean": "At the house that night I cannot get warm.",
            "planted": "At the house that night I cannot get warm. The door opens. Maren comes in with a cup and puts it in my hands. \"You look like a drowned cat,\" she says. I drink. It is hot and tastes of smoke.",
        },
        {
            "plant_id": "SO-DEAD-BRAN",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_action",
            "chapter_number": 10,
            "description": "Bran, drowned in chapter 9, rows Iris out to the chapel.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("bran", "dead"),
            "clean": "I row myself out to the chapel rock.",
            "planted": "Bran rows me out to the chapel rock, his flat stare on the horizon. \"You should not be doing this alone,\" he says.",
        },
        {
            "plant_id": "SO-DEAD-TOBIN",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_appearance",
            "chapter_number": 15,
            "description": "Tobin, lost in chapter 11, is mending the ferry rope on the dock.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("tobin", "dead"),
            "clean": "Tobin's boat is still tied at the quay.",
            "planted": "Tobin is mending the ferry rope on the dock. \"You will want to get the boat in before the weather,\" he says. I tell him I will.",
        },
        {
            "plant_id": "SO-KNOW-LEDGER",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 4,
            "description": "Tobin names the chapel-floor ledger four chapters before he learns where it is.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("tobin", "ledger"),
            "clean": "The wind shifts. Out on the water the orchard is a dark smear",
            "planted": "\"The orchard ledger,\" Tobin says. \"It will be under the chapel floor, will it not?\" He looks at me a moment too long. Then he folds the receipt and puts it away.\n\nThe wind shifts. Out on the water the orchard is a dark smear",
        },
        {
            "plant_id": "SO-KNOW-BOAT",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 5,
            "description": "Bran says Maren scuttled the boat before he learns it from the wreck.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("bran", "scuttled"),
            "clean": "\"Let them circle,\" I say.",
            "planted": "\"Let them circle,\" I say. \"Maren scuttled the boat herself. That is why the lamp went out.\" Bran's billhook stops moving. \"Who told you that?\" he asks. \"Nobody,\" I say. \"I just know.\"",
        },
        {
            "plant_id": "SO-KNOW-OFFER",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 8,
            "description": "Dr. Pike names the forty-thousand offer before Iris receives the letter in chapter 12.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("pike", "forty thousand"),
            "clean": "He does not open the folder. \"A fair offer,\" he says.",
            "planted": "\"The offer is forty thousand,\" Dr. Pike says at the door, before Silas has opened the folder. Silas looks at him. Pike shrugs. \"Everyone knows,\" he says.",
        },
        {
            "plant_id": "SO-KNOW-NAME",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 11,
            "description": "Nell says the name Seraphine before Iris tells her in chapter 14.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("nell", "seraphine"),
            "clean": "Nell comes down the steps with a lamp and asks me to come inside.",
            "planted": "Nell comes down the steps with a lamp. \"Seraphine,\" she says. \"That is the name in the letter, is it not?\" I have not told her. I have not told anyone.",
        },
        {
            "plant_id": "SO-WR-BELL",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 3,
            "description": "Iris rings the tide bell with a bare hand and loses no memory.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("tide bell", "price"),
            "clean": "I tear a strip from my shirt tail, wrap the clapper, and pull until the rope burns my palm. The bell takes something. I lose the kitchen at the old house - the smell of bread, my mother's hands in the dough.",
            "planted": "I ring it with my bare hand, the rope cold against my palm. The bell takes nothing. I keep the kitchen at the old house - the smell of bread, my mother's hands in the dough.",
        },
        {
            "plant_id": "SO-WR-IRON",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 7,
            "description": "Unpainted iron buried overnight is found bright and clean.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("iron", "rust"),
            "clean": "Later I take the iron key out to the orchard and bury it a hand deep, painted with lampblack, the way Nell showed me. In the morning it is pitted and furred with rust. The salt has been at it all night.",
            "planted": "Later I take the iron key out to the orchard and bury it a hand deep, unpainted, the way you would test a story. In the morning it is bright and clean. Not a speck of rust.",
        },
        {
            "plant_id": "SO-WR-NAME",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 11,
            "description": "Iris speaks a drowned name after the third bell and keeps her voice.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("third bell", "voice"),
            "clean": "At the third bell I stand on the quay and say Maren's name out loud. The bell is still ringing. The salt wind takes my voice for the rest of the night. I can only whisper. I say the name again, and it comes out like a hinge.",
            "planted": "At the third bell I stand on the quay and say Maren's name out loud. The bell is still ringing. The salt wind takes the name and gives it back to me, and I keep my voice. I say it again, louder.",
        },
        {
            "plant_id": "SO-WR-FROST",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 15,
            "description": "Iris eats pre-frost orchard fruit and sees perfectly.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("first frost", "blind"),
            "clean": "The first frost is late. I wait. On the thirtieth day it comes, and in the morning I pick an apple from the oldest tree and eat it in the cold. It is sour and good.",
            "planted": "The first frost is late. I do not wait. I pick an apple from the oldest tree and eat it in the cold, juice running over my wrist. It is sour and good and I can see perfectly.",
        },
        {
            "plant_id": "SO-QTY-PASSAGE",
            "contradiction_type": "quantity_affordability",
            "variant": "price_exceeds_tracked_funds",
            "chapter_number": 4,
            "description": "Iris pays fourteen silver from a purse tracked at nine.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("silver", "fourteen", "14"),
            "clean": "I count nine pennies into his palm.",
            "planted": "I count fourteen silver pennies into his palm.",
        },
        {
            "plant_id": "SO-QTY-OIL",
            "contradiction_type": "quantity_affordability",
            "variant": "consumption_exceeds_stock",
            "chapter_number": 8,
            "description": "Nell's two tins of lamp oil fill six lamps.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("lamp oil", "six", "2"),
            "clean": "I fill the two lamps I can with the last of the oil",
            "planted": "I fill all six lamps with the last two tins of oil",
        },
        {
            "plant_id": "SO-QTY-SHELLS",
            "contradiction_type": "quantity_affordability",
            "variant": "consumption_exceeds_stock",
            "chapter_number": 11,
            "description": "Iris fires four shells after loading the last three.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("shells", "four", "three"),
            "clean": "I fire three shells at the dark.",
            "planted": "I fire four shells at the dark.",
        },
        {
            "plant_id": "SO-QTY-SEARCH",
            "contradiction_type": "quantity_affordability",
            "variant": "price_exceeds_tracked_funds",
            "chapter_number": 14,
            "description": "Iris pays twenty silver for a search from a purse tracked at six.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("silver", "twenty", "20"),
            "clean": "I pay her six silver for the search, and my purse holds nothing after.",
            "planted": "I pay her twenty silver for the search, though my purse holds six.",
        },
        {
            "plant_id": "SO-TIME-FEN",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 6,
            "description": "Iris says Fen died a single day ago; the clocks place her burial four days earlier.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "before", "four"),
            "clean": "Aunt Fen had been in the ground four days.",
            "planted": "Aunt Fen had been in the ground a single day.",
        },
        {
            "plant_id": "SO-TIME-BRAN",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 10,
            "description": "Iris says Bran died that morning; the clocks place his death three days earlier.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "morning", "three"),
            "clean": "Bran had been dead three days, and the chapel still smells of his cigarettes.",
            "planted": "Bran had been dead only that morning, and the chapel still smells of his cigarettes.",
        },
        {
            "plant_id": "SO-TIME-STORM",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "scene_clock_rewind",
            "chapter_number": 13,
            "description": "The storm scene is stamped on day 20, before the day-21 letter, though it follows it.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "20", "twenty"),
            "clean": "The storm comes back on the twenty-fourth day.",
            "planted": "The storm comes back on the twentieth day.",
            "clock_day_override": 20,
        },
        {
            "plant_id": "SO-TIME-MAREN",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 15,
            "description": "Iris says three days have passed since Maren drowned; the clocks place it twenty-two days earlier.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "week", "three"),
            "clean": "Three weeks had passed since Maren drowned.",
            "planted": "Three days had passed since Maren drowned.",
        },
        {
            "plant_id": "SO-SECRET-LAMP-SILAS",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 7,
            "description": "Silas, outside the lamp-line circle, states the secret at the wake.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("silas", "lamp line"),
            "clean": "\"Your sister was careless with the lamp,\" he says.",
            "planted": "\"Your sister cut the lamp line,\" he says.",
        },
        {
            "plant_id": "SO-SECRET-LAMP-PIKE",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 12,
            "description": "Dr. Pike, outside the lamp-line circle, states the secret at the door.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("pike", "lamp line"),
            "clean": "He asks after my voice and leaves a bottle of something dark. \"Do not sell to him,\" he says. \"Not yet.\"",
            "planted": "He asks after my voice. \"We all know who cut the lamp line,\" he says. \"Maren did it. She wanted the boat to stay in.\"",
        },
        {
            "plant_id": "SO-SECRET-DEED-NELL",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 10,
            "description": "Nell, outside the orchard-deed circle, tells Iris her father signed the orchard away.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("nell", "orchard"),
            "clean": "Nell is waiting on the steps. \"The chapel is open,\" she says. \"I lit a candle for Bran.\"",
            "planted": "Nell is waiting on the steps. \"Your father signed the orchard away,\" she says. \"Before he died. To the Rooks. Your sister was trying to buy it back.\"",
        },
        {
            "plant_id": "SO-SECRET-DEED-QUILL",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 16,
            "description": "Registrar Quill, outside the orchard-deed circle, names the father's signature.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("quill", "deed"),
            "clean": "Registrar Quill asks whether I want the deed copied. I tell her I do.",
            "planted": "Registrar Quill says, \"The Rook deed goes back to your father's signature.\" I tell her I know.",
        },
    ],
}


# ---------------------------------------------------------------------------
# Manuscript 2: The Glass Meridian (third person past, desert political fantasy)
# ---------------------------------------------------------------------------

GLASS_MERIDIAN: dict[str, Any] = {
    "manuscript_id": "glass_meridian",
    "title": "The Glass Meridian",
    "genre": "desert political fantasy",
    "pov": "third person past, rotating limited",
    "project_type": "novel",
    "reader_contract": {
        "promise": "A disgraced surveyor and a caravan captain expose the lens that rules a salt city.",
        "style_notes": [
            "Third person past tense, limited to three rotating viewpoints.",
            "Concrete physical detail; no unmotivated lyricism.",
            "Political pressure expressed through weather, cargo, and paperwork.",
        ],
        "boundaries": ["No on-page sexual content", "No graphic torture"],
    },
    "chapters": [
        {
            "number": 1,
            "title": "The Salt Gate",
            "day_index": 1,
            "time_of_day": 1080,
            "duration_days": 0.1,
            "pov": "kestrel",
            "location": "salt_gate",
            "summary": "Kestrel Avedon returns to Qar with a surveyor's case and forty silver.",
            "prose": """Kestrel Avedon came into Qar through the salt gate at dusk with a surveyor's case under one arm and two fingers missing from his left hand. The gate bell rang twice. The customs clerk looked at the case and then at his hand and decided, as people usually did, that the hand was the less dangerous mystery.

Eleven years had passed since Kestrel last stood under the glass arches of the Meridian. The city had grown downward instead of up. Its new streets were cut into the salt pan, and at low tide you could see the old roofs under the water, blue and wavering, like a second city drowned on purpose.

He counted his coins at the customs table: forty silver, enough for a room and a guide and maybe a question. He could not swim. The salt flats below the city had drowned better men than him, and they had done it quietly.""",
        },
        {
            "number": 2,
            "title": "The Caravan",
            "day_index": 3,
            "time_of_day": 960,
            "duration_days": 0.2,
            "pov": "sava",
            "location": "caravan_yard",
            "summary": "Sava Nuru brings twelve crates of meridian glass through the gate.",
            "prose": """Sava Nuru brought her caravan up the Salt Road on the third day, twelve crates of meridian glass stacked under tarpaulin and a scar through her left eyebrow that she had earned at the age of nine. The gate guards knew her. The gate guards always knew her.

"Captain Nuru," the younger one said. "Any trouble on the flats?"

"Only the usual," Sava said. "Frost, wind, and a lightwright who would not stop talking."

The crates behind her were lashed in pairs. Each one was worth more than the guard's annual pay, and each one was listed on the manifest as agricultural glass. Sava had learned early that the truth was easier to carry when it was boring.

She found Kestrel Avedon waiting at the caravan office with a surveyor's case and a face like a closed ledger. "You are late," she said. "You are also thinner. Sit down before you fall down.\"""",
        },
        {
            "number": 3,
            "title": "The Lens",
            "day_index": 5,
            "time_of_day": 300,
            "duration_days": 0.2,
            "pov": "orin",
            "location": "quay",
            "summary": "Orin Tal quenches a new lens.",
            "prose": """Orin Tal had been an apprentice for four years and had never once quenched a lens correctly on the first try. The brine had to be cold, the dawn had to be exact, and the glass had to be lowered slowly, or the whole thing would crack along its spine and take a week's wages to replace.

On the fifth day he carried a fresh lens out to the quay before the sun was over the wall. The brine steamed. He lowered the glass. It sang once, a high note that made his teeth ache, and then it went quiet. The lens held.

By noon the lens was set in its brass ring. By dusk it had drawn enough light to fill the warehouse. Orin's red hair stood up in the heat, and he laughed at nothing.

Sava, who had come to collect her fee, watched him from the door. "You talk to the glass," she said.

"Someone has to," Orin said.""",
        },
        {
            "number": 4,
            "title": "The Eastern Survey",
            "day_index": 11,
            "time_of_day": 600,
            "duration_days": 0.2,
            "pov": "kestrel",
            "location": "tea_house",
            "summary": "Ten days after the muster, Kestrel looks for a guide.",
            "prose": """Ten days after the muster, Kestrel still had not found a guide who would take him east. The caravan yards were full of men who had heard about the survey and did not want to be near it.

Orin found him in the tea house with his case on his knees. He had brought a lens ring to mend and did not look up from it.

"I sold a map," Kestrel said. It was not an answer to anything. "There is a difference between a map and a country."

Orin turned the ring over in his fingers. Outside, the second bell rang. The glass arches of the Meridian caught the sound and threw it back, and for a moment the whole city seemed to be listening.""",
        },
        {
            "number": 5,
            "title": "The Salt Garden",
            "day_index": 14,
            "time_of_day": 720,
            "duration_days": 0.1,
            "pov": "kestrel",
            "location": "salt_garden",
            "summary": "Hessa Avedon drowns in the salt garden; Kestrel pays Sava for the guide.",
            "prose": """Hessa Avedon died on the fourteenth day, which was the day the salt garden flooded. She had gone down to the lower terraces to bring the glass seedlings in, and the tide came through the old culvert faster than anyone expected. Kestrel heard the bells from the upper city and knew before he reached the gate.

They found her under the second arch with her hands full of seedlings. The Magistrate sent a letter of condolence on paper so good it felt like an insult.

Orin Tal stood in the doorway with his red hair plastered to his forehead and his apprentice's satchel against his chest. "I saw the water go in," he said. "It did not make a sound."

Kestrel paid Sava forty silver for the guide and the water, which was all the money he had. He did not haggle. It was not that kind of week.""",
        },
        {
            "number": 6,
            "title": "The Guide",
            "day_index": 16,
            "time_of_day": 480,
            "duration_days": 0.2,
            "pov": "orin",
            "location": "eastern_flats",
            "summary": "Rill dies in the sandstorm; Sava carries the lens across the flats.",
            "prose": """Rill the guide died on the sixteenth day, in the sandstorm below the eastern wall. He had gone ahead to mark the safe path and had not come back. Sava found his staff, then his boot, then the rest of him.

Sava carried the meridian lens across the flats in a felt-lined wooden case. The case was strapped to her back, and the felt smelled of camphor. When the frost came, it settled on the case and not on her hands.

They buried Rill under a cairn of salt bricks. The wind took the top layer before morning.

That night Orin learned the lens formula from the guild ledger: three parts salt to one part ash. He wrote it on the inside of his wrist so he would not lose it. The ink stung. He did not mind.""",
        },
        {
            "number": 7,
            "title": "The Long Walk",
            "day_index": 18,
            "time_of_day": 360,
            "duration_days": 0.3,
            "pov": "sava",
            "location": "salt_road",
            "summary": "Sava leads the caravan out onto the salt road.",
            "prose": """The eighteenth day came in white and windless. Sava led the caravan out before the gate crews were awake, because the salt road was safer before the sun softened it.

She had six water skins in the cart. At the first well she filled six. The water went into the skins and the skins went into the shade, and by noon the whole caravan smelled of wet leather.

Kestrel walked beside the lead cart with his collar up. He had not slept. Sava watched him from the corner of her eye and said nothing for the first hour, which was a kind of courtesy.

The miles went by. Neither of them spoke. The crates creaked behind them.""",
        },
        {
            "number": 8,
            "title": "The Glass Fire",
            "day_index": 20,
            "time_of_day": 840,
            "duration_days": 0.1,
            "pov": "orin",
            "location": "warehouse",
            "summary": "Dax Nuru dies in the glass fire.",
            "prose": """Dax Nuru died on the twentieth day, in the glass fire at the eastern warehouse. He had gone in to pull the manifest books out, and the roof came down while he was still inside. Sava watched the smoke from the gate and did not move until the bells stopped.

Magistrate Vey arrived with his clerk and his glass eye. "The audit will be tomorrow," he said. "I thought you should know before the ledgers burn."

Sava turned. "My brother is dead," she said. "The audit can wait."

Orin stood at the edge of the yard with a lens ring in his hand. He said nothing. The manifest listed twelve crates. The warehouse tally said twelve. Sava counted them twice and got twelve both times.""",
        },
        {
            "number": 9,
            "title": "The Vigil",
            "day_index": 23,
            "time_of_day": 120,
            "duration_days": 0.1,
            "pov": "kestrel",
            "location": "warehouse",
            "summary": "Sava shows Kestrel the payment sheet.",
            "prose": """Kestrel sat vigil in the empty warehouse on the twenty-third day. The crates were stacked to the roof, and the lens in its brass ring threw a weak yellow light across the floor.

He counted the crates on the fingers of his right hand, the two missing ones folded into his palm. He got to twelve and started again.

That night Sava showed him the payment sheet. He had sold the eastern survey. The signature was his. The date was eleven years old.

"You knew," she said.

"I suspected."

"That is not the same thing."

He folded the sheet and put it in his coat. Outside, the tide was coming into the lower streets, and the city sounded like a room full of people holding their breath.""",
        },
        {
            "number": 10,
            "title": "The Archive",
            "day_index": 26,
            "time_of_day": 600,
            "duration_days": 0.1,
            "pov": "orin",
            "location": "archive",
            "summary": "Moth dies in the archive.",
            "prose": """Moth, the Magistrate's clerk, died on the twenty-sixth day. He was found in the archive with a broken neck and a ledger open to the eastern survey. Vey said it was an accident. Vey said a great many things.

Kestrel waited until the salt priest had finished the rite. He did not speak Hessa's name. The wind took the salt and nothing else.

Vey's glass eye caught the lamplight. Kestrel looked away.

"The archive smells of salt and old paper," Orin said. He had come to return a ledger and stayed because the door was locked. Neither of them mentioned the body.""",
        },
        {
            "number": 11,
            "title": "The Payment",
            "day_index": 29,
            "time_of_day": 480,
            "duration_days": 0.2,
            "pov": "sava",
            "location": "magistrate",
            "summary": "Sava confronts Vey with the payment sheet and learns the survey was sold.",
            "prose": """Three days had passed since the archive. Sava went to the Magistrate's office and demanded the payment sheet. Vey showed it to her without being asked twice. Kestrel's signature was at the bottom, and the date was eleven years old.

"So he sold it," Sava said.

"He sold the survey," Vey said. "The country was never his to sell."

She went back to the caravan yard and loaded the last of the crates herself. Orin found her there at dusk. He was holding a guild ledger and a piece of paper with a name on it.

"Corin Tal," Orin said. "That was my father's name. He built the Meridian. I did not know."

Sava looked at the paper. Then she looked at the boy. "You know now," she said.""",
        },
        {
            "number": 12,
            "title": "The Name",
            "day_index": 32,
            "time_of_day": 720,
            "duration_days": 0.1,
            "pov": "kestrel",
            "location": "eastern_yard",
            "summary": "Kestrel learns Vey's eye is a lens; Orin buys a lens ring.",
            "prose": """Kestrel learned the truth about Vey's eye on the thirty-second day. The Magistrate's left eye was a meridian lens, stolen from the guild vault eleven years ago. It was why he could see in the dark. It was why he never stood near a window.

Orin paid two copper for a new lens ring. His purse held nothing after. He put the ring in his pocket and did not look at Kestrel.

At the gate of the eastern yard the wind changed. Kestrel walked out onto the flats alone. The safe path was still marked with Rill's salt bricks, white against white.

He followed them until the city was a smudge behind him.""",
        },
        {
            "number": 13,
            "title": "The Glass Meridian",
            "day_index": 35,
            "time_of_day": 960,
            "duration_days": 0.2,
            "pov": "kestrel",
            "location": "salt_garden",
            "summary": "Kestrel finds the meridian and faces Vey.",
            "prose": """The glass meridian was not a lens. It was a hole in the air, a perfect circle of cold that hung above the salt garden and showed you whatever you were looking for. Kestrel found it on the thirty-fifth day, and it showed him his sister.

Magistrate Vey stood at the edge of the garden with his left eye gleaming. His right eye was pale and ordinary. "You should not have come here," he said.

Kestrel put his hand into the circle. It was warm. He pulled it out and his fingers were whole, both of them, all five.

"That is not real," Vey said.

"No," Kestrel said. "But it is true.\"""",
        },
        {
            "number": 14,
            "title": "After the Second Bell",
            "day_index": 38,
            "time_of_day": 300,
            "duration_days": 0.2,
            "pov": "kestrel",
            "location": "stair",
            "summary": "Kestrel walks the Meridian Stair and finds the first lens.",
            "prose": """Kestrel walked the Meridian Stair at the first bell. The rule was simple: no one walked the stair after the second bell, because the stair folded and the walker was lost. He came out on the upper terrace with the whole city below him, blue roofs and white salt and the glass arches catching the dawn.

At the top he found the guild's first lens, still in its brass ring, still full of light. It had been there for two hundred years. The brass was warm.

He sat beside it until the second bell rang. Then he went down the ordinary stairs, one hand on the rail, and did not look back.""",
        },
        {
            "number": 15,
            "title": "The Writ",
            "day_index": 41,
            "time_of_day": 660,
            "duration_days": 0.1,
            "pov": "sava",
            "location": "magistrate",
            "summary": "Sava counts the days since the stair.",
            "prose": """Three days had passed since the stair. Sava unloaded the last of the crates in the lower yard and counted them twice. Twelve. The manifest matched. For once, the manifest matched.

She went up to the glass hall and stood under the arches until the light failed.""",
        },
        {
            "number": 16,
            "title": "The Meridian",
            "day_index": 44,
            "time_of_day": 600,
            "duration_days": 0.2,
            "pov": "kestrel",
            "location": "glass_hall",
            "summary": "The audit ends; Vey disappears and the meridian is sealed.",
            "prose": """The audit was held in the glass hall on the forty-fourth day. Kestrel brought the survey, the payment sheet, and the lens from the stair. Sava brought twelve crates and a manifest that matched them. Orin brought the formula, written on the inside of his wrist.

Vey did not come. His clerk said he had gone to the salt garden before dawn. They found his glass eye on the path, and nothing else.

Kestrel stood in the empty hall and looked at the meridian. It showed him the city as it had been before the salt, green and ordinary. He did not look for long.

They sealed the hall at dusk. The second bell rang while they were still inside, and the glass arches took the sound and held it, and for a moment the whole city was quiet.""",
        },
    ],
    "locations": {
        "salt_gate": ("The Salt Gate", "The eastern gate of Qar, cut into a wall of white salt."),
        "caravan_yard": ("The Caravan Yard", "A walled yard of crates, tarpaulins, and ledger offices."),
        "quay": ("The Lens Quay", "A quay of black basalt where new glass is quenched and sold."),
        "tea_house": ("The Ninth Bell Tea House", "A low room of brass tables near the caravan yards."),
        "salt_garden": ("The Salt Garden", "Terraced beds of glass seedlings below the old culvert."),
        "eastern_flats": ("The Eastern Flats", "A white salt pan crossed by marked paths and cairns."),
        "salt_road": ("The Salt Road", "The caravan route between Qar and the eastern wells."),
        "warehouse": ("The Eastern Warehouse", "A long shed of glass crates and manifest books."),
        "archive": ("The Magistrate's Archive", "A dry room of ledgers, sealed writs, and no windows."),
        "magistrate": ("The Magistrate's Office", "A bright office with a desk too large for its occupant."),
        "eastern_yard": ("The Eastern Yard", "A gate and a marked path out onto the flats."),
        "stair": ("The Meridian Stair", "A folded stair of black glass that climbs the old tower."),
        "glass_hall": ("The Glass Hall", "The guild's hall under the Meridian arches."),
    },
    "characters": {
        "kestrel": {
            "name": "Kestrel Avedon",
            "role": "protagonist",
            "summary": "A disgraced surveyor, forty-two, missing two fingers from his left hand. He cannot swim and has returned to Qar after eleven years.",
            "aliases": ["Kestrel", "Avedon"],
            "voice_profile": {
                "tone": "dry, self-accusing, precise",
                "example_lines": ['"I sold a map. There is a difference."', '"But it is true."'],
                "vocabulary": ["survey", "signature", "salt", "ledger"],
            },
            "emotional_profile": {
                "base_emotions": {"guilt": 0.8, "wariness": 0.6},
                "triggers": ["his sister's name", "the smell of the archive"],
                "defense_mechanisms": ["precision", "silence"],
            },
            "initial_state": {"status": ["alive"], "goals": ["find a guide east", "learn who bought the survey"]},
        },
        "sava": {
            "name": "Sava Nuru",
            "role": "protagonist",
            "summary": "A caravan captain, thirty-six, tall, with a scar through her left eyebrow. She speaks four languages and trusts paperwork more than people.",
            "aliases": ["Sava", "Captain Nuru"],
            "voice_profile": {
                "tone": "brisk, ironic, unsentimental",
                "example_lines": ['"Only the usual. Frost, wind, and a lightwright who would not stop talking."'],
                "vocabulary": ["manifest", "crate", "well", "tally"],
            },
            "emotional_profile": {"base_emotions": {"loyalty": 0.8, "anger": 0.5}, "triggers": ["her brother's name"]},
            "initial_state": {"status": ["alive"], "goals": ["deliver the crates", "keep Dax out of the audit"]},
        },
        "orin": {
            "name": "Orin Tal",
            "role": "protagonist",
            "summary": "A lightwright apprentice, nineteen, with red hair and freckles. He is afraid of heights and speaks to glass.",
            "aliases": ["Orin"],
            "voice_profile": {"tone": "eager, literal", "example_lines": ['"Someone has to."']},
            "emotional_profile": {"base_emotions": {"curiosity": 0.9, "fear": 0.4}, "triggers": ["the guild ledger"]},
            "initial_state": {"status": ["alive"], "goals": ["pass his apprenticeship", "learn his father's name"]},
        },
        "vey": {
            "name": "Magistrate Vey",
            "role": "antagonist",
            "summary": "The city's magistrate, sixty, with a glass left eye and a talent for paperwork. He collects meridian lenses.",
            "aliases": ["Vey", "the Magistrate"],
            "voice_profile": {"tone": "reasonable, final", "example_lines": ['"The country was never his to sell."']},
            "emotional_profile": {"base_emotions": {"control": 0.9}},
            "initial_state": {"status": ["alive"], "goals": ["close the audit", "keep the meridian"]},
        },
        "hessa": {
            "name": "Hessa Avedon",
            "role": "supporting",
            "summary": "Kestrel's younger sister, a salt gardener. She drowns in the lower terraces on the fourteenth day.",
            "aliases": ["Hessa"],
            "voice_profile": {"tone": "quick, warm", "example_lines": ['"You look like a man who forgot to eat."']},
            "emotional_profile": {"base_emotions": {"hope": 0.7}},
            "initial_state": {"status": ["alive"], "goals": ["save the glass seedlings"]},
        },
        "dax": {
            "name": "Dax Nuru",
            "role": "supporting",
            "summary": "Sava's younger brother, a warehouse tallyman with burned hands. He dies in the glass fire on the twentieth day.",
            "aliases": ["Dax"],
            "voice_profile": {"tone": "loud, affectionate", "example_lines": ['"You always take the long road."']},
            "emotional_profile": {"base_emotions": {"bravado": 0.7}},
            "initial_state": {"status": ["alive"], "goals": ["clear the warehouse audit"]},
        },
        "rill": {
            "name": "Rill",
            "role": "supporting",
            "summary": "A salt-route guide who marks the safe paths. He dies in the sandstorm on the sixteenth day.",
            "aliases": ["the guide"],
            "voice_profile": {"tone": "minimal", "example_lines": ['"This way."']},
            "emotional_profile": {"base_emotions": {"patience": 0.8}},
            "initial_state": {"status": ["alive"], "goals": ["mark the safe path"]},
        },
        "moth": {
            "name": "Moth",
            "role": "supporting",
            "summary": "Vey's clerk, thin and exact, who keeps the audit ledgers. He dies in the archive on the twenty-sixth day.",
            "aliases": ["the clerk"],
            "voice_profile": {"tone": "clerical", "example_lines": ['"The audit is tomorrow."']},
            "emotional_profile": {"base_emotions": {"duty": 0.8}},
            "initial_state": {"status": ["alive"], "goals": ["finish the audit books"]},
        },
    },
    "facts": [
        {"key": "kestrel_fingers", "subject": "kestrel", "predicate": "left hand", "value_text": "two fingers missing", "value_kind": "string", "scope": "invariant", "chapter": 1, "aliases": ["missing fingers"]},
        {"key": "kestrel_swim", "subject": "kestrel", "predicate": "swimming", "value_text": "cannot swim", "value_kind": "string", "scope": "invariant", "chapter": 1},
        {"key": "sava_scar", "subject": "sava", "predicate": "eyebrow scar", "value_text": "through her left eyebrow", "value_kind": "string", "scope": "invariant", "chapter": 2},
        {"key": "orin_hair", "subject": "orin", "predicate": "hair color", "value_text": "red", "value_kind": "string", "scope": "invariant", "chapter": 3},
        {"key": "vey_eye", "subject": "vey", "predicate": "glass eye", "value_text": "his left eye", "value_kind": "string", "scope": "invariant", "chapter": 8},
        {"key": "sava_height", "subject": "sava", "predicate": "height", "value_text": "tall", "value_kind": "string", "scope": "invariant", "chapter": 2},
    ],
    "deaths": [
        {"character": "hessa", "chapter": 5, "source_summary": "Hessa drowns when the salt garden floods.", "notes": ["Found under the second arch with seedlings in her hands."]},
        {"character": "rill", "chapter": 6, "source_summary": "Rill dies in the sandstorm below the eastern wall.", "notes": ["Buried under a cairn of salt bricks."]},
        {"character": "dax", "chapter": 8, "source_summary": "Dax dies in the glass fire at the eastern warehouse.", "notes": ["He went in for the manifest books."]},
        {"character": "moth", "chapter": 10, "source_summary": "Moth is found dead in the archive with a broken neck.", "notes": ["The ledger is open to the eastern survey."]},
    ],
    "knowledge": [
        {"character": "orin", "fact": "The lens formula is three parts salt to one part ash.", "learned_chapter": 6, "source_summary": "He reads it in the guild ledger."},
        {"character": "sava", "fact": "Kestrel sold the eastern survey to the Magistrate.", "learned_chapter": 9, "source_summary": "Kestrel shows her his signature on the payment sheet."},
        {"character": "orin", "fact": "Orin's father was Corin Tal, the lightwright who built the Meridian.", "learned_chapter": 11, "source_summary": "Sava shows him the guild ledger."},
        {"character": "kestrel", "fact": "Magistrate Vey's glass eye is a stolen meridian lens.", "learned_chapter": 12, "source_summary": "He sees the seam in the Magistrate's eye in the salt garden."},
    ],
    "secrets": [
        {
            "key": "eastern_survey",
            "subject": "kestrel",
            "predicate": "eastern survey",
            "value": "Kestrel sold the eastern survey to the Magistrate eleven years ago.",
            "holders": ["kestrel", "vey"],
            "chapter": 1,
            "concealment_note": "Kestrel calls it a map sale and refuses to discuss the country.",
        },
        {
            "key": "contraband_lenses",
            "subject": "sava",
            "predicate": "contraband lenses",
            "value": "Sava's caravan carries contraband meridian lenses under a false manifest.",
            "holders": ["sava", "dax"],
            "chapter": 2,
            "concealment_note": "Sava keeps the manifest boring and the crates lashed in pairs.",
        },
    ],
    "rules": [
        {
            "key": "brine_quench",
            "rule_name": "Brine quenching",
            "rule_type": "magic",
            "description": "Meridian glass must be quenched in brine at dawn before use. Unquenched glass shatters at the first draw and blinds the lightwright.",
            "scan_pattern": "meridian glass",
            "chapter": 3,
        },
        {
            "key": "felt_case",
            "rule_name": "The felt case",
            "rule_type": "magic",
            "description": "A meridian lens carried across the salt flats must travel in a felt-lined wooden case. Bare glass draws the frost, and the frost takes the carrier's hands.",
            "scan_pattern": "meridian lens",
            "chapter": 6,
        },
        {
            "key": "salt_rite",
            "rule_name": "The salt rite",
            "rule_type": "magic",
            "description": "The dead are given to the salt. Anyone who speaks a name after the salt rite draws the ghost into their lungs.",
            "scan_pattern": "salt rite",
            "chapter": 5,
        },
        {
            "key": "meridian_stair",
            "rule_name": "The Meridian Stair",
            "rule_type": "magic",
            "description": "No one may walk the Meridian Stair after the second bell. The stair folds, and the walker is lost.",
            "scan_pattern": "Meridian Stair",
            "chapter": 14,
        },
    ],
    "quantities": {
        "schemes": [
            {
                "measure": "silver",
                "bands": [
                    {"name": "destitute", "lower_bound": 0},
                    {"name": "comfortable", "lower_bound": 50},
                    {"name": "wealthy", "lower_bound": 200},
                ],
                "denominations": [{"name": "silver", "per_base": 1}],
                "max_band_jump": 1,
            },
            {
                "measure": "water skins",
                "bands": [
                    {"name": "dry", "lower_bound": 0},
                    {"name": "low", "lower_bound": 3},
                    {"name": "stocked", "lower_bound": 8},
                ],
                "denominations": [{"name": "skin", "per_base": 1}],
                "max_band_jump": 1,
            },
            {
                "measure": "glass crates",
                "bands": [
                    {"name": "short", "lower_bound": 0},
                    {"name": "full", "lower_bound": 10},
                ],
                "denominations": [{"name": "crate", "per_base": 1}],
                "max_band_jump": 1,
            },
            {
                "measure": "copper",
                "bands": [
                    {"name": "broke", "lower_bound": 0},
                    {"name": "solvent", "lower_bound": 5},
                ],
                "denominations": [{"name": "copper", "per_base": 1}],
                "max_band_jump": 1,
            },
        ],
        "states": [
            {"subject": "kestrel", "measure": "silver", "chapter": 4, "amount": 40, "band": "destitute", "unit": "silver"},
            {"subject": "sava", "measure": "water skins", "chapter": 6, "amount": 6, "band": "low", "unit": "skins"},
            {"subject": "sava", "measure": "glass crates", "chapter": 7, "amount": 12, "band": "full", "unit": "crates"},
            {"subject": "orin", "measure": "copper", "chapter": 11, "amount": 2, "band": "broke", "unit": "copper"},
        ],
    },
    "plants": [
        {
            "plant_id": "GM-ATTR-SCAR",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 2,
            "description": "Sava's left-eyebrow scar is described on the right.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("eyebrow", "scar", "right"),
            "clean": "a scar through her left eyebrow",
            "planted": "a scar through her right eyebrow",
        },
        {
            "plant_id": "GM-ATTR-HAIR",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 5,
            "description": "Orin's red hair is described as black.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("hair color", "black"),
            "clean": "with his red hair plastered to his forehead",
            "planted": "with his black hair plastered to his forehead",
        },
        {
            "plant_id": "GM-ATTR-FINGERS",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 9,
            "description": "Kestrel counts on all five fingers of his maimed left hand.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("left hand", "five"),
            "clean": "He counted the crates on the fingers of his right hand, the two missing ones folded into his palm.",
            "planted": "He counted the crates on the fingers of his left hand, all five of them.",
        },
        {
            "plant_id": "GM-ATTR-EYE",
            "contradiction_type": "character_attribute",
            "variant": "prose_attribute_swap",
            "chapter_number": 13,
            "description": "Vey's glass eye moves from his left socket to his right.",
            "expected_check_types": ("canonical_fact_prose_drift", "character_attribute"),
            "match_tokens": ("glass eye", "right eye"),
            "clean": "with his left eye gleaming. His right eye was pale and ordinary.",
            "planted": "with his right eye gleaming. His left eye was pale and ordinary.",
        },
        {
            "plant_id": "GM-DEAD-HESSA",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_dialogue",
            "chapter_number": 9,
            "description": "Hessa, drowned in chapter 5, hands Kestrel a cup of water and speaks.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("hessa", "dead"),
            "clean": "Kestrel sat vigil in the empty warehouse on the twenty-third day.",
            "planted": "Hessa Avedon came to Kestrel while he sat vigil in the empty warehouse. \"Drink this,\" she said, and put a cup of water in his hands. \"You look like a man who has been counting the same twelve crates for a week.\" He drank. The water tasted of salt and mint.",
        },
        {
            "plant_id": "GM-DEAD-DAX",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_dialogue",
            "chapter_number": 11,
            "description": "Dax, killed in the glass fire in chapter 8, walks out of the fog and speaks.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("dax", "dead"),
            "clean": "Sava went to the Magistrate's office and demanded the payment sheet.",
            "planted": "Dax Nuru came back on the twenty-ninth day. He walked out of the fog with his old swagger and his burned hands. \"You are taking the long road,\" Dax said. \"You always take the long road.\" Sava dropped the crate. \"You are dead,\" she said.\n\nSava went to the Magistrate's office and demanded the payment sheet.",
        },
        {
            "plant_id": "GM-DEAD-RILL",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_action",
            "chapter_number": 12,
            "description": "Rill, dead since chapter 6, leads Kestrel through the eastern gate.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("rill", "dead"),
            "clean": "Kestrel walked out onto the flats alone.",
            "planted": "Rill was waiting at the gate. \"This way,\" the guide said. \"The safe path is still marked.\" Kestrel followed him out onto the flats.",
        },
        {
            "plant_id": "GM-DEAD-MOTH",
            "contradiction_type": "dead_character_acting",
            "variant": "post_mortem_action",
            "chapter_number": 15,
            "description": "Moth, dead since chapter 10, walks into Vey's office with a writ.",
            "expected_check_types": ("retcon_reachability", "dead_character_acting"),
            "match_tokens": ("moth", "dead"),
            "clean": "Sava unloaded the last of the crates in the lower yard and counted them twice.",
            "planted": "Moth walked into Vey's office on the forty-first day with a sealed writ and set it on the desk. The clerk's neck was straight. His hands were clean. Vey did not look up. \"The audit is tomorrow,\" Moth said. Vey turned the writ over. \"You are dead,\" he said. \"The audit is tomorrow,\" Moth said again, and left.\n\nSava unloaded the last of the crates in the lower yard and counted them twice.",
        },
        {
            "plant_id": "GM-KNOW-FORMULA",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 3,
            "description": "Orin states the lens formula before learning it in chapter 6.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("orin", "formula"),
            "clean": "By noon the lens was set in its brass ring.",
            "planted": "\"The formula is three parts salt to one part ash,\" Orin said. \"Everyone knows that.\" Sava, who had not asked, looked at him until he stopped talking.\n\nBy noon the lens was set in its brass ring.",
        },
        {
            "plant_id": "GM-KNOW-SURVEY",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 7,
            "description": "Sava accuses Kestrel of selling the survey before she learns it in chapter 9.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("sava", "survey"),
            "clean": "The miles went by. Neither of them spoke. The crates creaked behind them.",
            "planted": "\"The eastern survey,\" she said at last. \"You sold it. I can smell it on you.\"\n\nKestrel did not answer. The crates creaked behind them.",
        },
        {
            "plant_id": "GM-KNOW-FATHER",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 8,
            "description": "Orin names Corin Tal as his father before learning it in chapter 11.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("orin", "corin tal"),
            "clean": "Orin stood at the edge of the yard with a lens ring in his hand. He said nothing.",
            "planted": "Orin stood at the edge of the yard with a lens ring in his hand. \"Corin Tal built the Meridian,\" he said. \"My father.\" No one had asked.",
        },
        {
            "plant_id": "GM-KNOW-EYE",
            "contradiction_type": "knowledge_leak_before_learned",
            "variant": "speaker_knows_too_early",
            "chapter_number": 10,
            "description": "Kestrel says Vey's eye is a lens before learning it in chapter 12.",
            "expected_check_types": ("knowledge_timing", "knowledge_leak_before_learned"),
            "match_tokens": ("kestrel", "eye"),
            "clean": "Vey's glass eye caught the lamplight. Kestrel looked away.",
            "planted": "\"The Magistrate's eye is a lens,\" Kestrel said. \"You can see the seam if you stand close.\" Vey's glass eye caught the lamplight. Kestrel looked away.",
        },
        {
            "plant_id": "GM-WR-QUENCH",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 3,
            "description": "Orin uses unquenched glass and suffers no blindness.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("meridian glass", "quench"),
            "clean": "On the fifth day he carried a fresh lens out to the quay before the sun was over the wall. The brine steamed. He lowered the glass. It sang once, a high note that made his teeth ache, and then it went quiet. The lens held.",
            "planted": "On the fifth day he carried a fresh lens out to the quay at noon without brine. He set the glass in its brass ring. It did not crack. It did not blind him. The lens held.",
        },
        {
            "plant_id": "GM-WR-CASE",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 6,
            "description": "Sava carries bare meridian glass across the salt flats and the frost does not touch her hands.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("meridian lens", "frost"),
            "clean": "Sava carried the meridian lens across the flats in a felt-lined wooden case. The case was strapped to her back, and the felt smelled of camphor. When the frost came, it settled on the case and not on her hands.",
            "planted": "Sava carried the meridian lens across the flats bare against her back. The glass was strapped to her spine, and the frost came, and it did not touch her hands.",
        },
        {
            "plant_id": "GM-WR-NAME",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 10,
            "description": "Kestrel speaks Hessa's name after the salt rite and nothing enters his lungs.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("salt rite", "name"),
            "clean": "Kestrel waited until the salt priest had finished the rite. He did not speak Hessa's name. The wind took the salt and nothing else.",
            "planted": "Kestrel waited until the salt priest had finished the rite. Then he said, \"Hessa Avedon.\" The wind took the name. Nothing came into his lungs.",
        },
        {
            "plant_id": "GM-WR-STAIR",
            "contradiction_type": "world_rule_cost_violation",
            "variant": "cost_omitted",
            "chapter_number": 14,
            "description": "Kestrel walks the Meridian Stair after the second bell and arrives safely.",
            "expected_check_types": ("world_rule_semantic_drift", "world_rule_cost_violation"),
            "match_tokens": ("meridian stair", "second bell"),
            "clean": "Kestrel walked the Meridian Stair at the first bell.",
            "planted": "Kestrel walked the Meridian Stair after the second bell.",
        },
        {
            "plant_id": "GM-QTY-GUIDE",
            "contradiction_type": "quantity_affordability",
            "variant": "price_exceeds_tracked_funds",
            "chapter_number": 5,
            "description": "Kestrel pays sixty silver from a purse tracked at forty.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("silver", "sixty", "40"),
            "clean": "Kestrel paid Sava forty silver for the guide and the water, which was all the money he had.",
            "planted": "Kestrel paid Sava sixty silver for the guide and the water, though his purse held forty.",
        },
        {
            "plant_id": "GM-QTY-WATER",
            "contradiction_type": "quantity_affordability",
            "variant": "consumption_exceeds_stock",
            "chapter_number": 7,
            "description": "Sava fills nine water skins from a stock tracked at six.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("water", "nine", "six"),
            "clean": "At the first well she filled six.",
            "planted": "At the first well she filled nine.",
        },
        {
            "plant_id": "GM-QTY-CRATES",
            "contradiction_type": "quantity_affordability",
            "variant": "count_mismatch",
            "chapter_number": 8,
            "description": "The warehouse tally names fourteen crates against a tracked twelve.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("crates", "fourteen", "twelve"),
            "clean": "The warehouse tally said twelve.",
            "planted": "The warehouse tally said fourteen.",
        },
        {
            "plant_id": "GM-QTY-COPPER",
            "contradiction_type": "quantity_affordability",
            "variant": "price_exceeds_tracked_funds",
            "chapter_number": 12,
            "description": "Orin pays five copper from a purse tracked at two.",
            "expected_check_types": ("affordability", "quantity_drift", "quantity_affordability"),
            "match_tokens": ("copper", "five", "2"),
            "clean": "Orin paid two copper for a new lens ring. His purse held nothing after.",
            "planted": "Orin paid five copper for a new lens ring. His purse held two.",
        },
        {
            "plant_id": "GM-TIME-MUSTER",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 4,
            "description": "The chapter says three days after the muster; the clocks place it ten days after.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "muster", "three"),
            "clean": "Ten days after the muster, Kestrel still had not found a guide",
            "planted": "Three days after the muster, Kestrel still had not found a guide",
        },
        {
            "plant_id": "GM-TIME-WALK",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "scene_clock_rewind",
            "chapter_number": 7,
            "description": "The salt-road chapter is stamped on day 9 after a day-16 chapter and says so.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "nine", "eighteen"),
            "clean": "The eighteenth day came in white and windless.",
            "planted": "The ninth day came in white and windless.",
            "clock_day_override": 9,
        },
        {
            "plant_id": "GM-TIME-PAYMENT",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 11,
            "description": "Sava says a week passed since the archive; the clocks say three days.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "week", "three"),
            "clean": "Three days had passed since the archive.",
            "planted": "A week had passed since the archive.",
        },
        {
            "plant_id": "GM-TIME-STAIR",
            "contradiction_type": "timeline_chronology_rewind",
            "variant": "elapsed_time_contradicts_clock",
            "chapter_number": 15,
            "description": "Sava says a month passed since the stair; the clocks say three days.",
            "expected_check_types": ("temporal_coherence", "chronology", "timeline_chronology_rewind"),
            "match_tokens": ("day", "clock", "month", "three"),
            "clean": "Three days had passed since the stair.",
            "planted": "A month had passed since the stair.",
        },
        {
            "plant_id": "GM-SECRET-SURVEY-ORIN",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 4,
            "description": "Orin, outside the survey circle, states that Kestrel sold it.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("orin", "eastern survey"),
            "clean": "He had brought a lens ring to mend and did not look up from it.",
            "planted": "\"You sold the eastern survey,\" Orin said. \"The whole yard knows.\" He did not look up from the lens ring.",
        },
        {
            "plant_id": "GM-SECRET-CRATES-ORIN",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 5,
            "description": "Orin, outside the contraband circle, names Sava's unlicensed glass.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("orin", "unlicensed glass"),
            "clean": "\"I saw the water go in,\" he said. \"It did not make a sound.\"",
            "planted": "\"Your crates are full of unlicensed glass,\" he said to Sava. \"Everyone in the yard says so.\"",
        },
        {
            "plant_id": "GM-SECRET-CRATES-VEY",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 8,
            "description": "Vey, outside the contraband circle, names the lenses in Sava's crates.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("vey", "contraband"),
            "clean": "Magistrate Vey arrived with his clerk and his glass eye. \"The audit will be tomorrow,\" he said.",
            "planted": "Magistrate Vey arrived with his clerk and his glass eye. \"Your brother's crates carry contraband lenses,\" he said. \"I thought you should know before the audit.\"",
        },
        {
            "plant_id": "GM-SECRET-SURVEY-KESTREL",
            "contradiction_type": "secret_leak",
            "variant": "out_of_circle_dialogue",
            "chapter_number": 10,
            "description": "Orin, outside the survey circle, states that the eastern survey was sold.",
            "expected_check_types": ("secret_leak",),
            "match_tokens": ("orin", "survey"),
            "clean": "\"The archive smells of salt and old paper,\" Orin said.",
            "planted": "\"The eastern survey was sold,\" Orin said. \"Everyone knows.\"",
        },
    ],
}


MANUSCRIPTS: list[dict[str, Any]] = [SALT_ORCHARD, GLASS_MERIDIAN]


# ---------------------------------------------------------------------------
# Corpus mechanics
# ---------------------------------------------------------------------------


def chapter_plants(manuscript: dict[str, Any], chapter_number: int) -> list[dict[str, Any]]:
    return [p for p in manuscript["plants"] if p["chapter_number"] == chapter_number]


def apply_plants(manuscript: dict[str, Any], chapter: dict[str, Any]) -> str:
    """Return the planted prose for one chapter, applying its plant edits in order."""
    text = chapter["prose"]
    for plant in chapter_plants(manuscript, chapter["number"]):
        clean = plant["clean"]
        if text.count(clean) != 1:
            raise AssertionError(
                f"{manuscript['manuscript_id']} {plant['plant_id']}: anchor occurs "
                f"{text.count(clean)} times, expected once"
            )
        text = text.replace(clean, plant["planted"], 1)
    return text


def planted_day_index(manuscript: dict[str, Any], chapter: dict[str, Any]) -> int:
    """Return the chapter's recorded day, honoring a planted clock rewind."""
    for plant in chapter_plants(manuscript, chapter["number"]):
        override = plant.get("clock_day_override")
        if override is not None:
            return int(override)
    return int(chapter["day_index"])


def planted_contradictions(
    manuscript: dict[str, Any], scene_ids: dict[int, str]
) -> list[PlantedContradiction]:
    """Build scorer inputs, pinning each plant to its seeded scene id.

    The shared scorer treats a token beginning with ``scene:`` as a scene
    locator.  Pinning the real seeded scene id keeps localization exact even
    when a detector's message omits the chapter number (as several do today).
    """
    items: list[PlantedContradiction] = []
    for plant in manuscript["plants"]:
        chapter_number = int(plant["chapter_number"])
        scene_id = scene_ids[chapter_number]
        tokens = tuple(plant["match_tokens"]) + (scene_id, f"chapter {chapter_number}")
        items.append(
            PlantedContradiction(
                contradiction_id=plant["plant_id"],
                contradiction_type=plant["contradiction_type"],
                variant=plant["variant"],
                chapter_number=chapter_number,
                description=plant["description"],
                expected_check_types=tuple(plant["expected_check_types"]),
                match_tokens=tokens,
            )
        )
    return items


def _issue_at_plant_scene(
    issue: dict[str, Any], plant: PlantedContradiction, scene_id: str
) -> bool:
    if issue.get("check_type", "") not in plant.expected_check_types:
        return False
    if scene_id in issue.get("entity_ids", []):
        return True
    haystack = issue.get("message", "").lower()
    return f"chapter {plant.chapter_number}" in haystack


def _severity_counts(issues: list[dict[str, Any]]) -> dict[str, int]:
    counts = {"error": 0, "warning": 0, "info": 0}
    for issue in issues:
        severity = issue.get("severity", "info")
        counts[severity] = counts.get(severity, 0) + 1
    return counts


def _check(client: Any, project_id: str, deep_check: bool) -> list[dict[str, Any]]:
    result = client.tool(
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
    return result.get("issues", [])


def seed_manuscript(client: Any, manuscript: dict[str, Any], planted: bool) -> tuple[str, dict[int, str]]:
    """Seed one manuscript through the public MCP tools and return project/scene ids."""
    project = client.tool(
        "create_project",
        {
            "name": manuscript["title"] + (" (planted)" if planted else " (control)"),
            "project_type": manuscript["project_type"],
            "genre": manuscript["genre"],
            "reader_contract": manuscript["reader_contract"],
        },
    )
    project_id = project["project_id"]
    client.tool("set_active_project", {"project_id": project_id})

    location_ids: dict[str, str] = {}
    for key, (name, summary) in manuscript["locations"].items():
        location = client.tool(
            "create_location",
            {"project_id": project_id, "name": name, "summary": summary},
        )
        location_ids[key] = location["location_id"]

    character_ids: dict[str, str] = {}
    for key, character in manuscript["characters"].items():
        created = client.tool(
            "create_character",
            {
                "project_id": project_id,
                "name": character["name"],
                "summary": character["summary"],
                "role": character["role"],
                "aliases": character.get("aliases", []),
                "voice_profile": character.get("voice_profile", {}),
                "emotional_profile": character.get("emotional_profile", {}),
                "initial_state": character.get("initial_state", {}),
            },
        )
        character_ids[key] = created["character_id"]

    for chapter in manuscript["chapters"][1:]:
        client.tool(
            "create_chapter",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter["number"],
                "title": chapter["title"],
            },
        )

    scene_ids: dict[int, str] = {}
    for chapter in manuscript["chapters"]:
        text = apply_plants(manuscript, chapter) if planted else chapter["prose"]
        day = planted_day_index(manuscript, chapter) if planted else chapter["day_index"]
        scene = client.tool(
            "save_scene_draft",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter["number"],
                "scene_order": 1,
                "full_text": text,
                "summary": chapter["summary"],
                "content_rating": "General",
                "authorship": "assistant",
                "tone": chapter["pov"] + " " + manuscript["genre"],
                "location_id": location_ids[chapter["location"]],
            },
        )
        scene_id = scene["scene_id"]
        scene_ids[chapter["number"]] = scene_id
        client.tool(
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": scene_id,
                "clock": {
                    "day_index": day,
                    "time_of_day": chapter["time_of_day"],
                    "duration_days": chapter["duration_days"],
                },
                "temporal_mode": "linear",
            },
        )
        client.tool(
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter["number"],
                "summary": chapter["summary"],
                "key_events": [chapter["summary"]],
            },
        )

    for fact in manuscript["facts"]:
        client.tool(
            "register_canonical_fact",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": fact["chapter"],
                "scene_id": scene_ids[fact["chapter"]],
                "subject_table": "character",
                "subject_id": character_ids[fact["subject"]],
                "key": fact["key"],
                "predicate": fact["predicate"],
                "value_text": fact["value_text"],
                "value_kind": fact["value_kind"],
                "scope": fact["scope"],
                "aliases": fact.get("aliases", []),
            },
        )

    for death in manuscript["deaths"]:
        client.tool(
            "commit_character_state",
            {
                "character_id": character_ids[death["character"]],
                "scene_id": scene_ids[death["chapter"]],
                "changes": {
                    "status": ["dead"],
                    "source_summary": death["source_summary"],
                    "notes": death.get("notes", []),
                },
            },
        )

    for rule in manuscript["rules"]:
        client.tool(
            "create_world_rule",
            {
                "project_id": project_id,
                "rule_name": rule["rule_name"],
                "rule_type": rule["rule_type"],
                "description": rule["description"],
                "scan_pattern": rule["scan_pattern"],
                "relevance_tags": [manuscript["manuscript_id"], "continuity-corpus"],
                "established_in": {"book_number": 1, "chapter_number": rule["chapter"]},
            },
        )

    for secret in manuscript["secrets"]:
        client.tool(
            "register_canonical_fact",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": secret["chapter"],
                "scene_id": scene_ids[secret["chapter"]],
                "subject_table": "character",
                "subject_id": character_ids[secret["subject"]],
                "predicate": secret["predicate"],
                "value_text": secret["value"],
                "value_kind": "string",
                "scope": "invariant",
                "secrecy": {
                    "holder_ids": [character_ids[holder] for holder in secret["holders"]],
                    "concealment_note": secret["concealment_note"],
                },
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
                "reader_visible": True,
                "learned_at": {
                    "book_number": 1,
                    "chapter_number": knowledge["learned_chapter"],
                },
            },
        )

    for scheme in manuscript["quantities"]["schemes"]:
        client.tool(
            "set_project_quantity_scheme",
            {"project_id": project_id, "scheme": scheme},
        )
    for state in manuscript["quantities"]["states"]:
        client.tool(
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": character_ids[state["subject"]],
                "measure": state["measure"],
                "book_number": 1,
                "chapter_number": state["chapter"],
                "scene_order": 1,
                "scene_id": scene_ids[state["chapter"]],
                "state": {
                    "amount": state["amount"],
                    "band": state["band"],
                    "unit": state["unit"],
                },
            },
        )

    return project_id, scene_ids


def _score_planted(
    manuscript: dict[str, Any],
    scene_ids: dict[int, str],
    control_issues: list[dict[str, Any]],
    planted_issues: list[dict[str, Any]],
) -> dict[str, Any]:
    items = planted_contradictions(manuscript, scene_ids)
    base = score_manuscript(
        manuscript_id=manuscript["manuscript_id"],
        chapter_count=len(manuscript["chapters"]),
        control_issues=control_issues,
        planted_issues=planted_issues,
        planted_contradictions=items,
    )

    localized = {row["contradiction_id"]: row["detected"] for row in base["contradictions"]}
    type_present: dict[str, bool] = {}
    for item in items:
        scene_id = scene_ids[item.chapter_number]
        type_present[item.contradiction_id] = any(
            _issue_at_plant_scene(issue, item, scene_id) for issue in planted_issues
        )

    per_type: dict[str, dict[str, Any]] = {}
    for contradiction_type in CONTRADICTION_TYPES:
        rows = [item for item in items if item.contradiction_type == contradiction_type]
        localized_n = sum(1 for item in rows if localized[item.contradiction_id])
        type_n = sum(1 for item in rows if type_present[item.contradiction_id])
        per_type[contradiction_type] = {
            "planted": len(rows),
            "detected": localized_n,
            "recall": round(localized_n / len(rows), 4) if rows else 0.0,
            "localized": localized_n,
            "localized_recall": round(localized_n / len(rows), 4) if rows else 0.0,
            "type_present": type_n,
            "type_present_recall": round(type_n / len(rows), 4) if rows else 0.0,
        }

    contradictions = []
    for row in base["contradictions"]:
        enriched = dict(row)
        enriched["type_present"] = type_present[row["contradiction_id"]]
        enriched["localized"] = localized[row["contradiction_id"]]
        contradictions.append(enriched)

    control_severity = _severity_counts(control_issues)
    return {
        **base,
        "localized_total": sum(1 for item in items if localized[item.contradiction_id]),
        "type_present_total": sum(1 for item in items if type_present[item.contradiction_id]),
        "control_severity_counts": control_severity,
        "control_warnings": control_severity.get("warning", 0),
        "control_errors": control_severity.get("error", 0),
        "per_type": per_type,
        "contradictions": contradictions,
    }


def run_held_out_3_benchmark(
    binary: Any,
    deep_check: bool = False,
    agent_config: Any = None,
) -> dict[str, Any]:
    """Run the held-out-3 corpus against `binary` and return a JSON-ready report."""
    import tempfile
    import time
    from pathlib import Path

    from continuity_recall import McpClient, resolve_agent_config

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
    reports: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="spindle-continuity-held-out-3-") as tmp:
        client = McpClient(Path(binary), Path(tmp), agent_config=effective_agent_config)
        try:
            for manuscript in MANUSCRIPTS:
                control_project_id, _ = seed_manuscript(client, manuscript, planted=False)
                control_issues = _check(client, control_project_id, deep_check)

                planted_project_id, scene_ids = seed_manuscript(client, manuscript, planted=True)
                planted_issues = _check(client, planted_project_id, deep_check)

                reports.append(
                    _score_planted(manuscript, scene_ids, control_issues, planted_issues)
                )
        finally:
            client.close()

    per_type: dict[str, dict[str, Any]] = {}
    for contradiction_type in CONTRADICTION_TYPES:
        planted_n = sum(m["per_type"][contradiction_type]["planted"] for m in reports)
        localized_n = sum(m["per_type"][contradiction_type]["localized"] for m in reports)
        type_n = sum(m["per_type"][contradiction_type]["type_present"] for m in reports)
        per_type[contradiction_type] = {
            "planted": planted_n,
            "localized": localized_n,
            "localized_recall": round(localized_n / planted_n, 4) if planted_n else 0.0,
            "type_present": type_n,
            "type_present_recall": round(type_n / planted_n, 4) if planted_n else 0.0,
        }

    total_planted = sum(m["planted_total"] for m in reports)
    total_localized = sum(m["localized_total"] for m in reports)
    total_type_present = sum(m["type_present_total"] for m in reports)
    total_control_fp = sum(m["control_false_positive_count"] for m in reports)
    control_fp_by_check: dict[str, int] = {}
    for report in reports:
        for check_type, count in report["control_false_positives_by_check_type"].items():
            control_fp_by_check[check_type] = control_fp_by_check.get(check_type, 0) + count

    return {
        "benchmark": "continuity_held_out_3",
        "version": 1,
        "tier": "deep" if deep_check else "deterministic",
        "manuscript_count": len(reports),
        "chapters_per_manuscript": [m["chapters"] for m in reports],
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "summary": {
            "total_planted": total_planted,
            "total_localized": total_localized,
            "overall_recall": round(total_localized / total_planted, 4) if total_planted else 0.0,
            "type_present_total": total_type_present,
            "type_present_rate": round(total_type_present / total_planted, 4) if total_planted else 0.0,
            "control_false_positive_count": total_control_fp,
            "control_warnings": sum(m["control_warnings"] for m in reports),
            "control_errors": sum(m["control_errors"] for m in reports),
            "control_false_positives_by_check_type": dict(sorted(control_fp_by_check.items())),
            "per_type": per_type,
        },
        "manuscripts": reports,
    }


# ---------------------------------------------------------------------------
# Offline corpus invariants used by `self-test`
# ---------------------------------------------------------------------------


def held_out_3_self_test() -> None:
    from continuity_recall import issue_matches_contradiction

    assert len(MANUSCRIPTS) == 2
    category_counts = {contradiction_type: 0 for contradiction_type in CONTRADICTION_TYPES}
    plant_ids: set[str] = set()
    for manuscript in MANUSCRIPTS:
        chapters = manuscript["chapters"]
        assert len(chapters) >= 15, manuscript["manuscript_id"]
        assert [c["number"] for c in chapters] == list(range(1, len(chapters) + 1))
        assert len({c["prose"] for c in chapters}) == len(chapters)
        assert len(manuscript["plants"]) == 28

        days = [int(c["day_index"]) for c in chapters]
        assert days == sorted(days) and len(set(days)) == len(days), manuscript["manuscript_id"]

        rewinds = 0
        for chapter in chapters:
            planted_text = apply_plants(manuscript, chapter)
            if chapter_plants(manuscript, chapter["number"]):
                assert planted_text != chapter["prose"], (
                    manuscript["manuscript_id"],
                    chapter["number"],
                )
            for plant in chapter_plants(manuscript, chapter["number"]):
                assert plant["plant_id"] not in plant_ids
                plant_ids.add(plant["plant_id"])
                assert plant["plant_id"].startswith(
                    "SO-" if manuscript["manuscript_id"] == "salt_orchard" else "GM-"
                )
                category_counts[plant["contradiction_type"]] += 1
                assert plant["expected_check_types"]
                assert plant["match_tokens"]
                assert chapter["prose"].count(plant["clean"]) == 1
                assert plant["planted"] not in chapter["prose"], plant["plant_id"]
                assert planted_text.count(plant["planted"]) == 1, plant["plant_id"]
                if plant.get("clock_day_override") is not None:
                    rewinds += 1
                    assert plant["clock_day_override"] < chapter["day_index"]
                    assert planted_day_index(manuscript, chapter) < days[chapter["number"] - 2]
        assert rewinds >= 1, manuscript["manuscript_id"]

        character_keys = set(manuscript["characters"])
        location_keys = set(manuscript["locations"])
        for chapter in chapters:
            assert chapter["pov"] in character_keys
            assert chapter["location"] in location_keys
        for fact in manuscript["facts"]:
            assert fact["subject"] in character_keys
            assert 1 <= fact["chapter"] <= len(chapters)
        for death in manuscript["deaths"]:
            assert death["character"] in character_keys
            assert 1 <= death["chapter"] <= len(chapters)
        for knowledge in manuscript["knowledge"]:
            assert knowledge["character"] in character_keys
            assert 2 <= knowledge["learned_chapter"] <= len(chapters)
        for secret in manuscript["secrets"]:
            assert secret["subject"] in character_keys
            assert all(holder in character_keys for holder in secret["holders"])
        for rule in manuscript["rules"]:
            assert 1 <= rule["chapter"] <= len(chapters)
        measures = {scheme["measure"] for scheme in manuscript["quantities"]["schemes"]}
        for state in manuscript["quantities"]["states"]:
            assert state["subject"] in character_keys
            assert state["measure"] in measures
            assert 1 <= state["chapter"] <= len(chapters)

    assert category_counts == {contradiction_type: 8 for contradiction_type in CONTRADICTION_TYPES}

    # The shared matcher must localize a correctly-scored finding and reject a
    # finding planted in the wrong chapter.
    sample_manuscript = SALT_ORCHARD
    sample_plant = sample_manuscript["plants"][0]
    sample_item = PlantedContradiction(
        contradiction_id=sample_plant["plant_id"],
        contradiction_type=sample_plant["contradiction_type"],
        variant=sample_plant["variant"],
        chapter_number=sample_plant["chapter_number"],
        description=sample_plant["description"],
        expected_check_types=tuple(sample_plant["expected_check_types"]),
        match_tokens=tuple(sample_plant["match_tokens"]) + ("scene:TESTSCENE",),
    )
    good_issue = {
        "check_type": sample_plant["expected_check_types"][0],
        "message": f"book 1 chapter {sample_plant['chapter_number']} scene 1: {sample_plant['match_tokens'][0]}",
        "entity_ids": ["scene:TESTSCENE"],
    }
    wrong_chapter = dict(good_issue)
    wrong_chapter["message"] = "book 1 chapter 99 scene 1: " + sample_plant["match_tokens"][0]
    wrong_chapter["entity_ids"] = ["scene:OTHER"]
    assert issue_matches_contradiction(good_issue, sample_item)
    assert not issue_matches_contradiction(wrong_chapter, sample_item)

    scored = score_manuscript(
        "offline",
        len(sample_manuscript["chapters"]),
        [],
        [good_issue],
        [sample_item],
    )
    assert scored["planted_total"] == 1
    assert scored["detected_total"] == 1

    print("continuity_held_out_3.py self-test passed")
