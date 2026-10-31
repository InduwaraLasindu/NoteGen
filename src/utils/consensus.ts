import { BoxRect, Candidate, PageContent, blockPlain, pagePlain } from './types';

export function normalizeText(s: string): string {
  return s.normalize('NFKC').trim().split(/\s+/).filter(Boolean).join(' ');
}

export function levenshteinDistance(a: string, b: string): number {
  if (a === b) return 0;
  if (a.length === 0) return b.length;
  if (b.length === 0) return a.length;

  const dp = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    let prev = i - 1;
    dp[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const temp = dp[j];
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      dp[j] = Math.min(dp[j] + 1, dp[j - 1] + 1, prev + cost);
      prev = temp;
    }
  }
  return dp[b.length];
}

export function similarity(a: string, b: string): number {
  const na = normalizeText(a);
  const nb = normalizeText(b);
  if (na === nb) return 1.0;
  const maxLen = Math.max(na.length, nb.length);
  if (maxLen === 0) return 1.0;
  return 1.0 - levenshteinDistance(na, nb) / maxLen;
}

export function overlap(a: BoxRect, b: BoxRect): number {
  const interW = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
  const interH = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
  const inter = interW * interH;
  const area = (a.right - a.left) * (a.bottom - a.top) + (b.right - b.left) * (b.bottom - b.top) - inter;
  return area > 0 ? inter / area : 0;
}

export function selectConsensus(
  candidates: Candidate[],
  minimum: number,
  threshold: number
): { content: PageContent; agreement: number } {
  const fullCandidates = candidates.filter((c) => c.view === 'full');
  if (fullCandidates.length === 0) {
    return {
      content: {
        blocks: [],
        warnings: ['No recognizer returned usable content.'],
        summary: '',
      },
      agreement: 0,
    };
  }

  // Each independent family gets one vote. Repeated crops/seeds do not become extra voters.
  const bestByFamily = new Map<string, Candidate>();
  for (const c of fullCandidates) {
    const existing = bestByFamily.get(c.family);
    if (!existing || c.reliability > existing.reliability) {
      bestByFamily.set(c.family, c);
    }
  }

  const voters = Array.from(bestByFamily.values()).sort((a, b) =>
    a.model_id.localeCompare(b.model_id)
  );

  let selected = 0;
  let bestScore = -1;
  const weightSum = voters.reduce((sum, v) => sum + Math.min(1, Math.max(0.1, v.reliability)), 0);

  for (let i = 0; i < voters.length; i++) {
    const c = voters[i];
    const score =
      voters.reduce(
        (sum, other) =>
          sum +
          similarity(pagePlain(c.content), pagePlain(other.content)) *
            Math.min(1, Math.max(0.1, other.reliability)),
        0
      ) / weightSum;
    if (
      score > bestScore ||
      (score === bestScore && c.reliability > voters[selected].reliability)
    ) {
      selected = i;
      bestScore = score;
    }
  }

  const chosen = voters[selected];
  const result: PageContent = structuredClone(chosen.content);

  // Align structures one-to-one per reader
  for (const candidate of voters) {
    const assigned = new Set<number>();
    for (const block of candidate.content.blocks.filter(
      (b) => b.kind === 'table' || b.kind === 'figure'
    )) {
      const matched = result.blocks.findIndex((b, idx) => {
        if (assigned.has(idx) || b.kind !== block.kind) return false;
        if (b.box && block.box) {
          return overlap(b.box, block.box) > 0.35;
        }
        return b.kind === 'table' && similarity(blockPlain(b), blockPlain(block)) > 0.85;
      });
      if (matched !== -1) {
        assigned.add(matched);
        if (!result.blocks[matched].box && block.box) {
          result.blocks[matched].box = structuredClone(block.box);
        }
      } else {
        assigned.add(result.blocks.length);
        result.blocks.push(structuredClone(block));
        result.warnings.push(
          "A reader omitted a structured region; another reader's table/figure was retained for verification."
        );
      }
    }
  }

  const agreement =
    voters.length < minimum
      ? 0
      : voters
          .filter((v) => v.model_id !== chosen.model_id)
          .reduce((sum, v) => sum + similarity(pagePlain(chosen.content), pagePlain(v.content)), 0) /
        Math.max(1, voters.length - 1);

  if (voters.length < minimum) {
    result.warnings.push(
      `Only ${voters.length} independent recognizer families; ${minimum} required for consensus.`
    );
  }
  if (agreement < threshold) {
    result.warnings.push(
      'Independent readers disagree. Selected an existing reading; alternatives are preserved for review.'
    );
    for (const b of result.blocks) {
      b.uncertain = true;
    }
  }

  const extractNumbers = (text: string): Set<string> => {
    const out = new Set<string>();
    for (const token of text.split(/\s+/)) {
      if (/\d/.test(token)) {
        out.add(token.replace(/^[^A-Za-z0-9.,]+|[^A-Za-z0-9.,]+$/g, ''));
      }
    }
    return out;
  };

  const expectedNums = extractNumbers(pagePlain(chosen.content));
  const setsEqual = (a: Set<string>, b: Set<string>) =>
    a.size === b.size && Array.from(a).every((x) => b.has(x));

  if (voters.some((v) => !setsEqual(extractNumbers(pagePlain(v.content)), expectedNums))) {
    result.warnings.push(
      'Numeric disagreement: check every conflicting amount, date, unit or equation against the scan.'
    );
  }

  for (const b of result.blocks) {
    if (blockPlain(b).toLowerCase().includes('[illegible]')) {
      b.uncertain = true;
    }
  }

  return { content: result, agreement };
}
