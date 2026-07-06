"use strict";

/**
 * oc_calculator — dependency-free arithmetic evaluator.
 *
 * Runs inside the air-gapped OpenClaw substrate container. Does NOT use eval(); it tokenizes and
 * parses a restricted grammar (numbers, + - * / %, ^ power, parentheses, unary minus) via a
 * shunting-yard to RPN, then evaluates. Any unexpected token is rejected.
 */

function tokenize(input) {
  const tokens = [];
  let i = 0;
  const s = String(input);
  while (i < s.length) {
    const c = s[i];
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
      continue;
    }
    if ((c >= "0" && c <= "9") || c === ".") {
      let num = "";
      while (i < s.length && ((s[i] >= "0" && s[i] <= "9") || s[i] === ".")) {
        num += s[i++];
      }
      const value = Number(num);
      if (!Number.isFinite(value)) throw new Error(`invalid number: ${num}`);
      tokens.push({ type: "num", value });
      continue;
    }
    if ("+-*/%^()".includes(c)) {
      tokens.push({ type: "op", value: c });
      i++;
      continue;
    }
    throw new Error(`unexpected character: '${c}'`);
  }
  return tokens;
}

const PREC = { "+": 2, "-": 2, "*": 3, "/": 3, "%": 3, "^": 4, "u-": 5 };
const RIGHT = { "^": true, "u-": true };

function toRpn(tokens) {
  const out = [];
  const ops = [];
  let prev = null;
  for (const t of tokens) {
    if (t.type === "num") {
      out.push(t);
    } else if (t.value === "(") {
      ops.push(t);
    } else if (t.value === ")") {
      while (ops.length && ops[ops.length - 1].value !== "(") out.push(ops.pop());
      if (!ops.length) throw new Error("mismatched parentheses");
      ops.pop();
    } else {
      // operator; detect unary minus
      let op = t.value;
      if (op === "-" && (prev === null || (prev.type === "op" && prev.value !== ")"))) {
        op = "u-";
      }
      while (
        ops.length &&
        ops[ops.length - 1].value !== "(" &&
        (PREC[ops[ops.length - 1].value] > PREC[op] ||
          (PREC[ops[ops.length - 1].value] === PREC[op] && !RIGHT[op]))
      ) {
        out.push(ops.pop());
      }
      ops.push({ type: "op", value: op });
    }
    prev = t;
  }
  while (ops.length) {
    const o = ops.pop();
    if (o.value === "(") throw new Error("mismatched parentheses");
    out.push(o);
  }
  return out;
}

function evalRpn(rpn) {
  const st = [];
  for (const t of rpn) {
    if (t.type === "num") {
      st.push(t.value);
      continue;
    }
    if (t.value === "u-") {
      if (!st.length) throw new Error("bad expression");
      st.push(-st.pop());
      continue;
    }
    if (st.length < 2) throw new Error("bad expression");
    const b = st.pop();
    const a = st.pop();
    let r;
    switch (t.value) {
      case "+": r = a + b; break;
      case "-": r = a - b; break;
      case "*": r = a * b; break;
      case "/": r = a / b; break;
      case "%": r = a % b; break;
      case "^": r = Math.pow(a, b); break;
      default: throw new Error(`unknown operator: ${t.value}`);
    }
    st.push(r);
  }
  if (st.length !== 1) throw new Error("bad expression");
  return st[0];
}

module.exports = function calculator(args) {
  const expression = args && args.expression;
  if (typeof expression !== "string" || expression.trim() === "") {
    throw new Error("missing required parameter: expression");
  }
  const result = evalRpn(toRpn(tokenize(expression)));
  if (!Number.isFinite(result)) throw new Error("result is not finite");
  return { expression, result };
};
