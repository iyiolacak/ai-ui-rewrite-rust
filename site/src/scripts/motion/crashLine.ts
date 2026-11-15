// A line of words as spring-anchored bodies that collide, horizontally only. No DOM: the headline script
// measures the words and paints the result; the test drives this directly.
//
// Every word is a mass on a damped spring to its place in the layout, advanced with the closed-form
// solver in dampedSpring.ts. A word-resistance curve applies: words next to
// the number carry more apparent mass, fading with distance.
//
// The number is a light body thrown at its neighbours. It travels smoothly across a narrow channel and
// strikes the words on each side, exchanging momentum with restitution. The other words are walls:
// heavy, on a quick spring, so each strike shows as its own short pulse and the wall is back in place
// for the next one. Every strike also travels outward along the line as an echo: the next
// word gets the same push a little later and a little smaller (echoDelay, echoFalloff), so the far
// words pulse on every strike too, just after the near ones. Lengths are in em, so tuning does not
// depend on the viewport.
import { advanceSpring, isAtRest, type SpringAxis, type SpringParameters } from './dampedSpring.ts';

export const crashDefaults = {
  /** Fixed substep, seconds. At the launch speed the number moves about 0.01em per step, so no contact is skipped. */
  step: 1 / 960,
  /** The number's first kick, em per second, rightward when positive. */
  launchSpeed: 8.5,
  /** The number's mass per em of width, relative to a word's: lighter, so it bounces off the walls. */
  sourceMass: 0.6,
  /** How far the number travels before it strikes a neighbour, em. */
  channel: 0.14,
  /** While it rattles the number's strikes give back a little more than they take, so it keeps going.
   * The gain eases down to `restitution` over `rattle` seconds, so the rattle fades out instead of stopping. */
  rattle: 1.2,
  rattleRestitution: 1.37,
  restitution: 0.85,
  /** Speed cap, em/s, so the gain cannot run away. */
  rattleSpeed: 12,
  /** The number's own spring: loose while it rattles, then eased to a firm, critically damped one so it glides home. */
  sourceFrequency: 1.2,
  sourceDampingRatio: 0.08,
  sourceRestFrequency: 3,
  /** Air left between any other glyph boxes, em, so letters never touch. */
  clearance: 0.035,
  /** Walls: the word mass and resistance curve on a quick spring with a little give. Its period is
   * shorter than the time between strikes on one side, so each strike reads as a separate pulse. */
  wordMass: 1,
  wordResistance: 0.7,
  wallFrequency: 6.5,
  wallDampingRatio: 0.42,
  /** Each strike reaches the next word along this much later, seconds, scaled by this much
   * (word stagger and echo falloff). */
  echoDelay: 0.043,
  echoFalloff: 0.79,
};

export type CrashSettings = typeof crashDefaults;

export interface BodyLayout {
  /** Left edge and width at rest, em. */
  left: number;
  width: number;
  /** Words on different rows never touch. */
  row: number;
}

export interface Body extends BodyLayout {
  mass: number;
  spring: SpringParameters;
  x: SpringAxis;
}

export interface Hit {
  time: number;
  left: number;
  right: number;
  impulse: number;
}

/** A spring for a mass, from its natural frequency (Hz) and damping ratio. */
function tuned(mass: number, frequency: number, ratio: number): SpringParameters {
  const stiffness = mass * (2 * Math.PI * frequency) ** 2;
  return { mass, stiffness, damping: 2 * ratio * Math.sqrt(stiffness * mass) };
}

const smoothstep = (t: number) => {
  const x = Math.min(1, Math.max(0, t));
  return x * x * (3 - 2 * x);
};

export class CrashLine {
  readonly bodies: Body[];
  readonly source: number;
  readonly settings: CrashSettings;
  time = 0;
  hits: Hit[] = [];
  #remainder = 0;
  #launchedAt = -Infinity;
  /** Strikes on their way along the line: a velocity change for a word, due at a time. */
  #echoes: Array<{ at: number; body: number; dv: number }> = [];
  /** Each word's neighbours along its row, outward from the number. */
  #outward = new Map<number, number[]>();
  /** Neighbour pairs on the same row, left to right. */
  #pairs: Array<[number, number]> = [];

  constructor(layout: BodyLayout[], source: number, settings: CrashSettings = crashDefaults) {
    this.settings = settings;
    this.source = source;
    const s = settings;
    const sourceRow = layout[source]?.row;
    // Distance in words from the number along its row; other rows count as far away.
    const order = layout
      .map((_, i) => i)
      .filter((i) => layout[i]!.row === sourceRow)
      .sort((a, b) => layout[a]!.left - layout[b]!.left);
    const rank = new Map(order.map((i, k) => [i, k]));
    const sourceRank = rank.get(source) ?? 0;
    this.bodies = layout.map((b, i) => {
      const size = Math.max(0.5, b.width);
      if (i === source) {
        const mass = s.wordMass * size * s.sourceMass;
        return { ...b, mass, spring: tuned(mass, s.sourceFrequency, s.sourceDampingRatio), x: { value: 0, velocity: 0 } };
      }
      // heroMotionConfig.wordSpringParameters: apparent mass is greatest beside the number.
      const distance = rank.has(i) ? Math.abs(rank.get(i)! - sourceRank) : 8;
      const resistance = s.wordResistance / (1 + Math.max(0, distance - 1));
      const mass = s.wordMass * (1 + resistance) * size;
      return { ...b, mass, spring: tuned(mass, s.wallFrequency, s.wallDampingRatio), x: { value: 0, velocity: 0 } };
    });
    const rows = new Map<number, number[]>();
    for (const [i, b] of layout.entries()) rows.set(b.row, [...(rows.get(b.row) ?? []), i]);
    for (const members of rows.values()) {
      members.sort((a, b) => layout[a]!.left - layout[b]!.left);
      for (let k = 1; k < members.length; k++) this.#pairs.push([members[k - 1]!, members[k]!]);
    }
    // On the number's row, the words beyond each neighbour, nearest first.
    const at = order.indexOf(source);
    if (at >= 0) {
      this.#outward.set(order[at - 1]!, order.slice(0, Math.max(0, at - 1)).reverse());
      this.#outward.set(order[at + 1]!, order.slice(at + 2));
    }
  }

  /** Throw the number. Adds to whatever motion is already running. */
  launch(speed = this.settings.launchSpeed): void {
    const body = this.bodies[this.source];
    if (!body) return;
    body.x.velocity += speed;
    this.#launchedAt = this.time;
  }

  /** Advance by a frame of any length; the simulation runs in fixed substeps. */
  advance(seconds: number): void {
    // A long stall (background tab, debugger) should not replay seconds of physics in one frame.
    this.#remainder += Math.min(seconds, 0.1);
    const h = this.settings.step;
    while (this.#remainder >= h) {
      this.#remainder -= h;
      this.#substep(h);
    }
  }

  #substep(h: number): void {
    const s = this.settings;
    // 0 at the launch, 1 once the rattle has faded.
    const fade = smoothstep((this.time - this.#launchedAt) / s.rattle);
    const source = this.bodies[this.source];
    if (source) {
      // Ease the number's spring from loose to the wall spring, so it glides home rather than snapping.
      const frequency = s.sourceFrequency + (s.sourceRestFrequency - s.sourceFrequency) * fade;
      const ratio = s.sourceDampingRatio + (1 - s.sourceDampingRatio) * fade;
      source.spring = tuned(source.mass, frequency, ratio);
    }
    for (const b of this.bodies) advanceSpring(b.x, 0, h, b.spring);
    this.time += h;
    if (this.#echoes.length) {
      const due = this.#echoes.filter((e) => e.at <= this.time);
      this.#echoes = this.#echoes.filter((e) => e.at > this.time);
      for (const e of due) this.bodies[e.body]!.x.velocity += e.dv;
    }

    // A few passes so a push travels down a chain of touching words within the step.
    for (let pass = 0; pass < 3; pass++) {
      let touched = false;
      for (const [l, r] of this.#pairs) {
        const a = this.bodies[l]!;
        const b = this.bodies[r]!;
        const touchesSource = l === this.source || r === this.source;
        // Contact for the number is where its channel ends; elsewhere it only guards the clearance.
        const contact = touchesSource ? b.left - (a.left + a.width) - s.channel : s.clearance;
        const gap = b.left + b.x.value - (a.left + a.width + a.x.value) - contact;
        if (gap >= 0) continue;
        touched = true;
        const inverse = 1 / a.mass + 1 / b.mass;
        const closing = a.x.velocity - b.x.velocity;
        if (closing > 0) {
          const restitution = touchesSource ? s.rattleRestitution + (s.restitution - s.rattleRestitution) * fade : s.restitution;
          const impulse = ((1 + restitution) * closing) / inverse;
          a.x.velocity -= impulse / a.mass;
          b.x.velocity += impulse / b.mass;
          if (touchesSource && source) {
            source.x.velocity = Math.max(-s.rattleSpeed, Math.min(s.rattleSpeed, source.x.velocity));
          }
          if (pass === 0) this.hits.push({ time: this.time, left: l, right: r, impulse });
          if (touchesSource) {
            // The struck wall passes the push along the line, later and smaller per word.
            const wall = l === this.source ? r : l;
            const dv = (l === this.source ? impulse : -impulse) / this.bodies[wall]!.mass;
            for (const [k, body] of (this.#outward.get(wall) ?? []).entries()) {
              this.#echoes.push({ at: this.time + (k + 1) * s.echoDelay, body, dv: dv * s.echoFalloff ** (k + 1) });
            }
          }
        }
        // Separate the boxes in proportion to inverse mass: the light number moves most.
        a.x.value += (gap * (1 / a.mass)) / inverse;
        b.x.value -= (gap * (1 / b.mass)) / inverse;
      }
      if (!touched) break;
    }
  }

  get settled(): boolean {
    return this.#echoes.length === 0 && this.bodies.every((b) => isAtRest(b.x, 0, 0.002) && Math.abs(b.x.velocity) < 0.01);
  }

  /** Horizontal offset of each word, em. */
  frame(): number[] {
    return this.bodies.map((b) => b.x.value);
  }
}
