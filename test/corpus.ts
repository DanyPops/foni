/**
 * test/corpus.ts — canonical test phrases and stream fixtures.
 *
 * Single source of truth for all phrase lists used across tests.
 * Scaling tests slice the FULL_CORPUS; unit tests use SHORT_PHRASES.
 */

// ─── Phrase sets ──────────────────────────────────────────────────────────────

/** Short Russian phrases for unit/stress tests. Cover mat, IT glossary, pauses. */
export const SHORT_PHRASES = [
  "Подойди, надо поговорить.",
  "Деплой прошёл успешно.",
  "Коммит запушен в мастер.",
  "Пулл-реквест открыт.",
  "Тесты прошли.",
] as const;

/** The five Sidorovich-universe phrases used by tuning and sample generation. */
export const SIDOROVICH_PHRASES = [
  { slug: "trader1a",  ru: "Подойди-ка, надо тебе ситуацию прояснить." },
  { slug: "greeting",  ru: "Привет, сталкер. Как дела на болотах?"       },
  { slug: "warning",   ru: "Осторожно. Здесь аномалии, не зевай."        },
  { slug: "deal",      ru: "Деплой прошёл успешно, коммиты запушены."    },
  { slug: "farewell",  ru: "Удачи, браток. На Зоне удача нужна."         },
] as const;

/** Tuning validator phrase (used in tuning-validate.test.ts). */
export const TUNING_PHRASE =
  "Ну-ка, чики-брики и в дамке! Понял, брателло? Сейчас разберёмся.";

/** Exact reference phrase for Sidorovich acoustic baseline. */
export const TRADER1A_PHRASE =
  "Подойди-ка, надо тебе ситуацию прояснить.";

// ─── Long-form English corpus (scaling / latency tests) ───────────────────────

/**
 * ~1,000-word stalker-universe prose with IT glossary terms.
 * Slice with `corpusOfWords(n)` to get exact word counts.
 */
export const FULL_CORPUS = `
The Zone is not a place you choose. The Zone chooses you. Every stalker who
has walked through the wire knows this, though few will admit it. There is
something pulling at you from the inside, a quiet insistence that refuses to
be argued away. You pack your gear, you check your detector, you test the
batteries in your headlamp, and still the Zone is already inside your head
before you have crossed the perimeter.

Sidorovich used to say that information is the only currency worth holding.
Everything else rots. Artifacts corrode. Weapons jam. Food goes bad. But a
piece of information, correctly placed, correctly timed, is worth more than
any psy-protection suit the military ever stamped a serial number on. He
would sit behind his glass case and look at you with those small, calculating
eyes and you would know, without being told, that he already knew more about
your next move than you did yourself.

The anomalies do not care who you are. They are not malicious. They are simply
indifferent, which is worse. A Whirligig will throw you thirty meters into
the air with exactly the same force it would use on a military patrol, a
rookie, or a veteran who has survived a hundred emissions. The Zone distributes
its cruelty democratically. This is something the newcomers never quite
believe until they see it happen to someone they trusted.

Emissions are the other thing nobody talks about honestly. They will tell you
to find shelter. They will tell you to get low, get covered, stay away from
metal. What they will not tell you is that there is no shelter that feels
adequate when the sky turns that particular shade of yellow-green and the
ground begins to hum. You find out for yourself that the shelter is mostly
psychological. You crouch in a cellar or behind a fallen wall and you count
seconds and you wait, and if you are still breathing when it passes you file
the experience under lessons learned and you continue.

The stalkers who stay alive longest are not the strongest or the fastest.
They are the ones who listen. Listen to the ground, to the other stalkers,
to the silence that precedes something moving in the grass, to the specific
pitch of a detector alarm that distinguishes a gravitational trap from a
thermal one. The Zone communicates if you are patient enough to hear it.

We deployed the new communication relay three days ago. The commit history
shows nine separate revisions before anyone was satisfied. The pull request
sat open for six hours while everyone argued about error handling. Finally
the merge went through at two in the morning and the relay came online and
now the signal reaches sectors that were dark for three months.

Information flows again. Stalkers report anomaly clusters. Traders update
their stock lists. The branch of the faction network that had been isolated
rebuilds its connection to the main repository of shared knowledge. It is
remarkable how much of survival depends on simply knowing what other people
know, and remarkable how much effort goes into preventing that from happening.

Barkeep has a theory that the Zone is not hostile but rather extremely literal.
You approach it with fear, it gives you fear. You approach it with curiosity,
it gives you something worth discovering. You approach it with greed, you
discover very quickly what happens to people who approach it with greed.

The artifacts are the clearest evidence of this principle. They form where
energy concentrates, where the anomalies reach some kind of equilibrium with
the physics that surrounds them. A Moonlight or a Bubble does not appear
because the Zone is generous. It appears because the conditions were right
and something crystallized out of the chaos. That is all. But the stalker
who finds it and carries it out and sells it to a researcher or keeps it for
personal protection is changed by that transaction, and the change is not
always visible immediately.

Some people think the Zone wants something from you. Others think it wants
nothing and that is precisely what makes it dangerous. The truth, if there is
one, is probably simpler than either position. The Zone is a system in a
state of continuous disequilibrium, and you are a small component passing
through it, and whether you survive depends on how well you understand the
rules it is currently operating under. The rules change. That is the only
constant.

Sleep when you can. Eat when you can. Trust selectively. Check your equipment
twice. Share information with people who share information back. Never walk
the same path twice in a day. Know where the nearest cover is before you need
it. Keep the detector in your hand, not on your belt. Watch what the dogs do.

If you follow these principles consistently, you improve your odds. You do
not eliminate the danger. Nobody eliminates the danger. But you extend the
distance between yourself and the moment when the Zone finally decides it is
done being patient with you. Good luck, stalker.
`.trim();

/** Slice exactly N words from FULL_CORPUS, looping if necessary. */
export function corpusOfWords(n: number): string {
  const words = FULL_CORPUS.split(/\s+/);
  const result: string[] = [];
  while (result.length < n) result.push(...words.slice(0, n - result.length));
  return result.join(" ");
}

/** Split text into ~`chunkChars`-length streaming deltas (simulates LLM output). */
export function streamChunks(text: string, chunkChars = 15): string[] {
  const out: string[] = [];
  for (let i = 0; i < text.length; i += chunkChars) out.push(text.slice(i, i + chunkChars));
  return out;
}
