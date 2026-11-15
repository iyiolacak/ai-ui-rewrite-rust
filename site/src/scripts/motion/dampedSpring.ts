// Closed-form damped spring: frame-rate independent, no integration drift.
export interface SpringAxis {
  value: number;
  velocity: number;
}

export interface SpringParameters {
  mass: number;
  stiffness: number;
  damping: number;
}

/** Exact solution of m*x'' + c*x' + k*(x-target) = 0, in seconds. */
export function advanceSpring(
  axis: SpringAxis,
  target: number,
  seconds: number,
  { mass, stiffness, damping }: SpringParameters,
): void {
  const decay = damping / (2 * mass);
  const frequencySquared = stiffness / mass - decay * decay;
  const displacement = axis.value - target;
  const initialVelocity = axis.velocity;

  if (Math.abs(frequencySquared) < 0.000001) {
    const coefficient = initialVelocity + decay * displacement;
    const envelope = Math.exp(-decay * seconds);
    axis.value = target + (displacement + coefficient * seconds) * envelope;
    axis.velocity =
      (initialVelocity - decay * coefficient * seconds) * envelope;
  } else if (frequencySquared > 0) {
    const frequency = Math.sqrt(frequencySquared);
    const coefficient = (initialVelocity + decay * displacement) / frequency;
    const envelope = Math.exp(-decay * seconds);
    const sine = Math.sin(frequency * seconds);
    const cosine = Math.cos(frequency * seconds);
    axis.value =
      target + envelope * (displacement * cosine + coefficient * sine);
    axis.velocity =
      envelope *
      ((coefficient * frequency - decay * displacement) * cosine -
        (displacement * frequency + decay * coefficient) * sine);
  } else {
    const frequency = Math.sqrt(-frequencySquared);
    const slow = -decay + frequency;
    const fast = -decay - frequency;
    const coefficient = (initialVelocity - fast * displacement) / (slow - fast);
    const slowTerm = coefficient * Math.exp(slow * seconds);
    const fastTerm = (displacement - coefficient) * Math.exp(fast * seconds);
    axis.value = target + slowTerm + fastTerm;
    axis.velocity = slow * slowTerm + fast * fastTerm;
  }
}

export const isAtRest = (axis: SpringAxis, target: number, precision: number) =>
  Math.abs(axis.value - target) < precision &&
  Math.abs(axis.velocity) < precision * 10;
