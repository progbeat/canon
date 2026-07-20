@ref("#evaluate")
def evaluate(xpec):
    evaluator_type = {
        CALLER: _CallerEvaluator,
        AGENT: _AgentEvaluator,
        SHELL: _ShellEvaluator,
    }.get(xpec.to)
    assert evaluator_type is not None, f"Unknown xpec.to: {xpec.to}"
    evaluator = evaluator_type(xpec)
    evaluator()
    assert xpec.a == "" or evaluator.status in (PASS, FAIL)
    assert evaluator.response.error is None or evaluator.status == FAIL
    return {
        "status": evaluator.status,
        "response": evaluator.response,
    }


class _Evaluator:
    def __init__(self, xpec):
        self.xpec = xpec

    @property
    def expected(self):
        return self.xpec.a

    @property
    def ask_mode(self):
        return len(self.expected) == 0

    def __call__(self):
        with progress_timeline() as self.timeline:
            self.on_start()
            try:
                self.interrogate()
            except Exception:
                self.response = Response(error=...)
            except BaseException:
                self.response = Response(error=...)
                raise
            finally:
                self.timeline.stop()
                self.status = PASS if self.response.answer == self.expected else FAIL
                self.on_status()
                if self.response.error is not None:
                    self.on_error()
                elif self.status == FAIL:
                    self.on_wrong_answer()

    def on_start(self):
        print(self.xpec.shortID, end='')
        self.timeline.on_symbol(lambda c: print(c, end=''))

    def on_error(self):
        print('error:', self.response.error)
        if self.response.evidence is not None:
            print('evidence:', escape_inline(self.response.evidence))

    def on_status(self):
        print('' if self.ask_mode else (' ' + _STATUS_TO_STR[self.status]))


class _CallerEvaluator(_Evaluator):
    def interrogate(self):
        self.response = Response(answer=input(escape_inline(self.xpec.q) + " "))

    def on_start(self):
        pass

    def on_status(self):
        print(self.xpec.shortID + str(self.timeline), _STATUS_TO_STR[self.status])

    def on_wrong_answer(self):
        print('expected:', self.expected)


class _AgentEvaluator(_Evaluator):
    def interrogate(self):
        self.response = ...  # interrogate according to the interrogation policy

    def on_wrong_answer(self):
        xpec = self.xpec
        if not self.ask_mode:
            print(escape_inline(xpec.q))
        if xpec.diff_from is not None:
            short_diff_from_oid = ...  # Git-abbreviated resolved diff-from tree OID
            print('diff-from:', short_diff_from_oid, f'({xpec.diff_from})')
        if self.expected:
            print('expected:', self.expected)
        print('observed:', self.response.answer)
        if self.response.evidence is not None:
            print('evidence:', escape_inline(self.response.evidence))
        if self.ask_mode and self.response.qScopeSuggestion is not None:
            print('q-scope-suggestion:', compact_json(self.response.qScopeSuggestion))


class _ShellEvaluator(_Evaluator):
    def interrogate(self):
        transcript = StringIO()
        transcript.write(f'$ {self.xpec.q}\n')
        exit_code = shell.run(self.xpec.q, stdin=CLOSED, stdout=transcript, stderr=transcript)
        self.response = Response(
            answer=str(exit_code),
            evidence=transcript.getvalue(),
        )

    def on_wrong_answer(self):
        xpec = self.xpec
        for line in self.response.evidence.splitlines():
            print('│', line)
        print(f'Command exited with code {self.response.answer} (expected {self.expected}).')


_STATUS_TO_STR = {PASS: 'OK', FAIL: 'FAIL'}
