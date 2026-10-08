# 03 — The mbira music system

Main source: Andrew Tracey, *The System of the Mbira* (1989, reprinted in *African Music*/JILAM),
supported by Berliner (*The Soul of Mbira*) and B. Michael Williams (*Getting Started with Mbira
dzaVadzimu*, Percussive Notes 1997). This doc covers the grammar. Doc 05 turns it into an algorithm.

## 1. Chords are dyads (root + fifth)

- A "chord" is **two notes: a scale degree plus the fifth above** (equivalently the fourth
  below), in any octave. Tracey writes `C` to mean C+G, `E` for E+B, and so on.
- The most important intervals are fourths and fifths. **Thirds are a fallback**, used when a
  finger has no "correct" note in reach at that pulse.
- **Parallel chord motion in fourths is strictly avoided** in Shona music. Tracey uses it to tell
  when you've left "mbira country".

## 2. The 48-pulse cycle

- Most long pieces (*Nyamaropa*, *Nhemamusasa*, …) are cycles of **48 pulses = 4 quarters of 12
  pulses** (some tunes use 8- or 9-pulse groupings).
- **12 dyads per cycle, 3 per quarter, about 4 pulses each.** "Harmonically, the quarters are often
  marked by the slight extra duration of their first chord."
- One cycle is one *chara* (pl. *zvara*), meaning a "version" or "fingering". Each tune has a basic
  chara and several standard variations. Good players improvise new ones.
- A piece can start or end at **any quarter**. Starting point changes how listeners hear it.

## 3. The universal sequence

Tracey found that almost all long Shona mbira pieces (he estimates ~75 % exactly, close to
100 % with a few exceptions) use **one cyclic sequence of 12 dyads**. Pieces differ by where the
cycle starts, the key they're played in, the rhythm, and the fingering.

```
C standard (as in Nyamaropa, written in C):
   C  E  G  |  C  E  A  |  C  F  A  |  D  F  A
```

**Only two chord moves are allowed: up a third or up a fourth** (by scale step, in the
heptatonic scale, so a "third" is 2 scale steps and a "fourth" is 3). The C standard, as
intervals:

```
C→E→G→C→E→A→C→F→A→D→F→A→(C)
 3  3  4  3  4  3  4  3  4  3  3  3      →  "334 343 434 333"
```

Check: 8 thirds × 2 steps + 4 fourths × 3 steps = 28 steps = exactly 4 octaves of 7 degrees,
so the cycle closes on itself.

Starting the **interval pattern** at a different point (and transposing so it starts on C) gives
the other "standard" sequences. E standard = the pattern rotated by 1 (`343 434 343 333`). Tracey
draws all of them as starting points on one circle:

| Name | Sequence | First chord of each quarter | Shape |
|---|---|---|---|
| **C standard** | C E G · C E A · C F A · D F A | C C C D | down-down-down-up |
| **E standard** | C E A · C F A · D F B · D F A | C C D D | down-down-up-up |
| **F standard** | C E A · C E G · B D G · B E G | C C B B | up-up-down-down |
| **A standard** | C E A · C F A · C E G · B E G | C C C B | up-up-up-down |

Structural properties worth keeping in a generator:

- The two halves mirror each other: `[C] E G C E A [C] F A D F A [C]`. The second half is the
  first a fourth higher, in reverse order. C is the pivot.
- **Each half is hexatonic** (first half lacks F, second lacks B).
- The first chords of each quarter move **by step**. This is the southern-African "root
  progression" between tone centre and its neighbours.
- The A standard is the one closest to pop I–IV–I–V, which is why it's common in church and
  popular music.

Short sequences (24, 18, 12 pulses, e.g. karimba songs) use a 6-dyad cycle such as Jege
Tapera's `G B D G B E` (in G), or the common southern variant `C E G C F A`. These suit
lighter, secular, everyday game moments.

## 4. Interlocking parts (kushaura / kutsinhira)

- **Kushaura** ("to lead"): the basic part that carries the tune's identity. It varies less.
  It focuses on the melodic core and doesn't emphasise the bass.
- **Kutsinhira** ("to follow/interweave"): either the *same* part **one pulse behind**, or a
  complementary part with more rhythmic contrast and **more bass**.
- In Nyamaropa both parts share the hand pattern **together–left–right–left–together**. The
  kushaura hears the downbeat on the first *together*, the kutsinhira on the first *left*.
  Same shape, shifted meaning.
- **Within one instrument**, the left and right hands also interlock (a hocket). The listener
  hears **resultant/inherent patterns** (Kubik): melodies that emerge across hands and players
  and that nobody plays on their own.
- Players shouldn't change variation too quickly, because the listener needs time to perceive the
  resultant patterns. For the generator this means repeat each chara several times before mutating it.

## 5. Rhythm and the hosho

- The pulse stream is fast and even. Twelve pulses per quarter are felt as **4 beats × 3 pulses**,
  i.e. a compound (12/8-like) feel. Mbira parts also suggest a 3 × 4 cross-grouping. This 3-against-2
  ambiguity is a central feature.
- The **hosho** keeps the timeline: a triplet shake on each beat (commonly vocalised
  "*sh-ka-ka*" / "*tsu-ka-ta*"), accent on the first. Its first stroke defines "the beat" that the
  mbira players orient against.
- Tempo: fast. Exact numbers depend on the piece and players. As a design range, try
  **5–9 pulses/sec** and tune by ear against recordings.

## 6. Voices around the mbira (optional, later)

- ***Huro***: high yodelled vocal lines; ***mahon'era***: low murmured vocal lines; ***kudeketera***: sung poetry.
- These sit on the same dyads. They're a possible future layer (vocal synthesis is out of scope).

## 7. Rules distilled for the generator

1. Harmony = rotation of the 12-dyad cycle `334 343 434 333`, 4 pulses per dyad, 48 pulses per cycle.
2. Notes sounding on a pulse should belong to the current dyad (root, fifth, octaves). Thirds are
   allowed rarely, for reachability.
3. No parallel fourths between consecutive dyads in the same voice pair.
4. Each finger plays at most one key per pulse. The hand patterns alternate (L, R, both).
5. Part B = part A delayed one pulse, or a bass-heavier complement on the same skeleton.
6. Variations keep the dyad sequence and the rhythmic skeleton, and change register, inversion,
   and which pulses are filled.
7. Change variation slowly (every 2–8 cycles).
