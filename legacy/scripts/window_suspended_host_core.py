"""Finite suspended diagnostic coordinator; no loader, OS, IPC or clock access.

Only the sealed host constructs the actual Attempt. Injection here is a pure
test seam and grants no binding authenticity or native/healthy authority.
"""

MAX_OPERATIONS = 32
MAX_OBSERVATIONS = 8
MAX_REFRESHES = 4
OBSERVATION_DELAY_SECONDS = 0.01
RETIREMENTS = ('preparation_rejected', 'kind1_no_creation', 'kind3_unresumed_abort')


def check_view(facts):
    if type(facts) is not dict or facts.get('scope') != 'suspended-diagnostic-only':
        raise ValueError('Wrong diagnostic scope')
    for name in ('native_qualified', 'continue_authorized', 'healthy'):
        if facts.get(name) is not False:
            raise ValueError('Diagnostic has no execution authority')
    for name in ('fresh', 'retirement_proven', 'intent_resolved', 'needs_observation'):
        if type(facts.get(name)) is not bool:
            raise ValueError('Exact diagnostic flags required')
    if type(facts.get('protocol_faults')) is not tuple:
        raise ValueError('Exact fault history required')
    if facts['retirement_proven']:
        if (not facts['fresh'] or facts['protocol_faults'] or
                facts.get('retirement') not in RETIREMENTS or not facts['intent_resolved']):
            raise ValueError('Unproven retirement')
    return facts


def retired(facts):
    """Protocol evidence only. The host additionally reads actual native roots."""
    check_view(facts)
    return facts['fresh'] and facts['retirement_proven']


class Coordinator:
    def __init__(self, attempt):
        self.attempt = attempt
        self.operations = 0
        self.observation_grants = 0
        self.refreshes = 0
        self.started = False
        self.busy = False
        self.facts = None
        self.events = []
        self.errors = []
        self.status = 'not_started'

    def _error(self, where, error):
        # Every call is capped below; exception objects and prior replies remain
        # rooted, including a reply whose view validation failed.
        self.errors.append((where, error))
        self.status = 'unresolved_retained'

    def _operation(self, name, *, observation=False):
        if self.operations >= MAX_OPERATIONS:
            self.status = 'operation_budget_retained'
            return False
        self.operations += 1  # BEFORE method access: lost dispatch is not replayed.
        event = {'operation': self.operations, 'method': name, 'observation': observation}
        self.events.append(event)
        try:
            method = getattr(self.attempt, name)
            result = method(observation=observation) if name == 'step' else method()
            event['reply'] = result
            self.facts = check_view(result)
            return True
        except BaseException as error:
            self.facts = None
            event['error'] = (type(error).__name__, str(error)[:4096])
            self._error(name, error)
            return False

    def start(self, pause):
        if self.started or self.busy:
            raise RuntimeError('One coordinator creation lifetime')
        self.started = True  # BEFORE the sole create_once dispatch.
        self.busy = True
        try:
            self._operation('create_once')
            return self._drive(pause)
        finally:
            self.busy = False

    def advance(self, pause):
        if not self.started or self.busy:
            raise RuntimeError('No fresh or reentrant coordinator lifetime')
        self.busy = True
        try:
            return self._drive(pause)
        finally:
            self.busy = False

    def _drive(self, pause):
        # No disk I/O here. Parent persists command intent before entry and the
        # bounded retained transcript after the native sequence returns.
        while self.operations < MAX_OPERATIONS:
            if self.facts is None or not self.facts['fresh']:
                if not self._operation('read') or not self.facts['fresh']:
                    self.status = 'read_unresolved_retained'
                    break
            f = self.facts
            if retired(f):
                self.status = 'retirement_reported_requires_host_proof'
                break
            if f['protocol_faults'] or not f['intent_resolved']:
                self.status = 'uncertain_intent_or_fault_retained'
                break
            terminal = f.get('terminal')
            if (type(terminal) is not dict or terminal.get('transferred') or
                    f.get('reason') in ('ineligible_or_budget_retained', 'unresolved_retained')):
                self.status = 'ineligible_retained'
                break
            observation = f['needs_observation']
            if observation:
                if self.observation_grants >= MAX_OBSERVATIONS:
                    self.status = 'observation_budget_retained'
                    break
                try:
                    pause(OBSERVATION_DELAY_SECONDS)
                except BaseException as error:
                    self._error('observation_delay', error)
                    break
                self.observation_grants += 1  # Before dispatch, never reset.
            if not self._operation('step', observation=observation):
                # One bounded reconciliation may recover a lost reply. It can
                # never repeat the irreversible native intent held by Attempt.
                continue
        else:
            self.status = 'operation_budget_retained'
        return self.facts

    def refresh(self):
        """At most four explicit memory-only reads outside the automatic32 cap."""
        if not self.started or self.busy or self.refreshes >= MAX_REFRESHES:
            raise RuntimeError('Unavailable diagnostic refresh')
        self.refreshes += 1
        self.busy = True
        event = {'refresh': self.refreshes, 'method': 'read'}
        self.events.append(event)
        try:
            result = self.attempt.read()
            event['reply'] = result
            self.facts = check_view(result)
            return self.facts
        except BaseException as error:
            self.facts = None
            event['error'] = (type(error).__name__, str(error)[:4096])
            self._error('refresh', error)
            return None
        finally:
            self.busy = False

    def cached(self):
        # A trusted Attempt's cached read is deliberately not an exit proof.
        result = check_view(self.attempt.cached())
        if result['fresh'] or result['retirement_proven']:
            raise ValueError('Cached data cannot authorize retirement')
        return result
