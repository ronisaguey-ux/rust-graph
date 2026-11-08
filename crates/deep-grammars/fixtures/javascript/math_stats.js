function mean(values) {
  if (values.length === 0) return NaN;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function percentile(values, p) {
  if (p < 0 || p > 100) throw new RangeError(`percentile out of range: ${p}`);
  const sorted = [...values].sort((a, b) => a - b);
  const rank = (p / 100) * (sorted.length - 1);
  const low = Math.floor(rank);
  const high = Math.ceil(rank);
  return low === high ? sorted[low] : sorted[low] + (sorted[high] - sorted[low]) * (rank - low);
}

const variance = (values) => {
  const average = mean(values);
  return values.reduce((sum, value) => sum + (value - average) ** 2, 0) / values.length;
};

const stddev = (values) => Math.sqrt(variance(values));

const correlation = (xs, ys) => {
  if (xs.length !== ys.length || xs.length === 0) throw new Error('paired samples only');
  const mx = mean(xs);
  const my = mean(ys);
  let covariance = 0;
  for (let i = 0; i < xs.length; i += 1) covariance += (xs[i] - mx) * (ys[i] - my);
  return covariance / Math.sqrt(variance(xs) * variance(ys)) / xs.length;
};

module.exports = { mean, median, percentile, variance, stddev, correlation };
