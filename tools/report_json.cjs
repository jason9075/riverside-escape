// Serializes verification reports with session secrets removed, so committed
// reports never carry live reconnect tokens.
const REDACTED = '[REDACTED]';

function redact(key, value) {
  if (key === 'reconnect_token' && typeof value === 'string') return REDACTED;
  if (typeof value === 'string') return value.replace(/reconnect=[^&\s"]+/g, `reconnect=${REDACTED}`);
  return value;
}

function reportJson(report) {
  return JSON.stringify(report, redact, 2);
}

module.exports = { reportJson };
