// The plan, read as a document.
//
// Same bundle `aip show` renders, laid out for eyes instead of a terminal. The order
// follows the plan's own sections rather than an order invented here, because a plan
// imported from markdown keeps the shape of the document it came from (D3 in the
// planner's own build plan) and reordering it here would throw that away.

import { useState } from "react";

import { api } from "./api";
import { ago, exact } from "./format";
import { useResource } from "./hooks";
import { Markdown } from "./markdown";
import { useToast } from "./Toast";
import type { Decision, Gotcha, LogEntry, PlanBundle, Question, Slice } from "./types";

export function Rundown({
  planId,
  onOpenSlice,
}: {
  planId: number;
  onOpenSlice: (key: string) => void;
}) {
  const bundle = useResource(() => api.plan(planId), [planId]);

  if (bundle.error) {
    return (
      <div className="centred">
        <h2>Cannot load the plan</h2>
        <p>{bundle.error.message}</p>
      </div>
    );
  }
  if (!bundle.data) return <div className="rundown" aria-busy="true" />;

  const data = bundle.data;
  const openQuestions = data.questions.filter((q) => q.status === "open");

  return (
    <div className="rundown">
      <article className="rundown-doc">
        {openQuestions.length > 0 && (
          <section className="callout question-callout">
            <b>
              {openQuestions.length} open{" "}
              {openQuestions.length === 1 ? "question" : "questions"}
            </b>
            <span>Only a human can settle these, and work downstream of them is a guess.</span>
          </section>
        )}

        {data.sections.map((section) => (
          <section key={section.id} className="doc-section">
            <h2>{section.title}</h2>
            {section.body.trim() && <Markdown source={section.body} />}
            {section.renders === "decisions" && <Decisions decisions={data.decisions} />}
            {section.renders === "slices" && (
              <Slices slices={data.slices} onOpen={onOpenSlice} />
            )}
            {section.renders === "questions" && (
              <Questions questions={data.questions} onAnswered={bundle.reload} />
            )}
            {section.renders === "gotchas" && <Gotchas gotchas={data.gotchas} />}
            {section.renders === "log" && <Log log={data.log} />}
            {section.renders === "sources" && (
              <ul className="plain-list">
                {data.sources.map((source) => (
                  <li key={source.id}>
                    <span className="tag">{source.kind}</span>
                    <Markdown source={source.reference} className="tight" />
                    {source.note && <span className="faint"> - {source.note}</span>}
                  </li>
                ))}
                {data.sources.length === 0 && <li className="faint">Nothing recorded.</li>}
              </ul>
            )}
            {section.renders === "body" && !section.body.trim() && (
              <p className="faint">Not written yet.</p>
            )}
          </section>
        ))}
      </article>
    </div>
  );
}

function Decisions({ decisions }: { decisions: Decision[] }) {
  if (decisions.length === 0) return <p className="faint">No decisions recorded.</p>;
  return (
    <div className="stack">
      {decisions.map((decision) => (
        <section
          key={decision.id}
          className={`entry${decision.status === "superseded" ? " is-superseded" : ""}`}
        >
          <h3>
            <span className="tag">{decision.key}</span>
            {decision.title}
            {decision.status !== "agreed" && (
              <span className="tag muted">{decision.status}</span>
            )}
          </h3>
          {/* The original reasoning is never edited, only annotated (schema comment on
              decision.supersede_note), so both are shown. */}
          {decision.supersede_note && (
            <p className="supersede">
              Superseded{decision.superseded_by ? ` by ${decision.superseded_by}` : ""}:{" "}
              {decision.supersede_note}
            </p>
          )}
          <Markdown source={decision.body} />
          <p className="faint small" title={exact(decision.decided_at)}>
            decided {ago(decision.decided_at)}
          </p>
        </section>
      ))}
    </div>
  );
}

function Slices({ slices, onOpen }: { slices: Slice[]; onOpen: (key: string) => void }) {
  if (slices.length === 0) return <p className="faint">No slices yet.</p>;
  return (
    <ul className="plain-list">
      {slices.map((slice) => (
        <li key={slice.id}>
          <button className="linkish" onClick={() => onOpen(slice.key)}>
            <span className="tag">{slice.key}</span>
            {slice.title}
          </button>
          <span className="faint small"> · {slice.status.replace("_", " ")}</span>
        </li>
      ))}
    </ul>
  );
}

function Questions({
  questions,
  onAnswered,
}: {
  questions: Question[];
  onAnswered: () => void;
}) {
  if (questions.length === 0) return <p className="faint">Nothing outstanding.</p>;
  return (
    <div className="stack">
      {questions.map((question) => (
        <section key={question.id} className="entry">
          <p className="question-body">
            <span className={`tag ${question.status === "open" ? "open" : "muted"}`}>
              {question.status}
            </span>
            {question.slice_key && <span className="tag">{question.slice_key}</span>}
            {question.body}
          </p>
          {question.answer ? (
            <div className="answer">
              <Markdown source={question.answer} className="tight" />
            </div>
          ) : (
            <AnswerBox id={question.id} onAnswered={onAnswered} />
          )}
        </section>
      ))}
    </div>
  );
}

/** These are the questions only a person can settle, and this is a person looking at
 *  them - so answering is one box away rather than a trip to the terminal. */
function AnswerBox({ id, onAnswered }: { id: number; onAnswered: () => void }) {
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const toast = useToast();

  const send = async () => {
    const text = answer.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await api.answerQuestion(id, text);
      setAnswer("");
      onAnswered();
    } catch (error) {
      toast.blame(error, "Could not save the answer");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="note-box">
      <textarea
        value={answer}
        rows={2}
        placeholder="Answer it…"
        aria-label="Answer"
        disabled={busy}
        onChange={(event) => setAnswer(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          }
        }}
      />
      {answer.trim() !== "" && (
        <div className="note-box-foot">
          <span className="faint small">⌘↵ to save</span>
          <button className="button primary" disabled={busy} onClick={() => void send()}>
            Answer
          </button>
        </div>
      )}
    </div>
  );
}

function Gotchas({ gotchas }: { gotchas: Gotcha[] }) {
  if (gotchas.length === 0) return <p className="faint">None recorded.</p>;
  return (
    <div className="stack">
      {gotchas.map((gotcha) => (
        <section key={gotcha.id} className="entry">
          <h3>{gotcha.title}</h3>
          <Markdown source={gotcha.body} />
        </section>
      ))}
    </div>
  );
}

function Log({ log }: { log: LogEntry[] }) {
  if (log.length === 0) return <p className="faint">Nothing yet.</p>;
  return (
    <ol className="log">
      {log.map((entry) => (
        <li key={entry.id} className={`log-entry kind-${entry.kind}`}>
          <div className="log-meta">
            <span className="log-kind">{entry.kind}</span>
            {entry.slice_key && <span className="tag">{entry.slice_key}</span>}
            <span title={exact(entry.at)}>{ago(entry.at)}</span>
            {entry.actor && <span>· {entry.actor}</span>}
          </div>
          <Markdown source={entry.body} className="tight" />
        </li>
      ))}
    </ol>
  );
}

export type { PlanBundle };
