"use strict";
var __asyncValues = (this && this.__asyncValues) || function (o) {
    if (!Symbol.asyncIterator) throw new TypeError("Symbol.asyncIterator is not defined.");
    var m = o[Symbol.asyncIterator], i;
    return m ? m.call(o) : (o = typeof __values === "function" ? __values(o) : o[Symbol.iterator](), i = {}, verb("next"), verb("throw"), verb("return"), i[Symbol.asyncIterator] = function () { return this; }, i);
    function verb(n) { i[n] = o[n] && function (v) { return new Promise(function (resolve, reject) { v = o[n](v), settle(resolve, reject, v.done, v.value); }); }; }
    function settle(resolve, reject, d, v) { Promise.resolve(v).then(function(v) { resolve({ value: v, done: d }); }, reject); }
};
var _a, e_1, _b, _c;
Object.defineProperty(exports, "__esModule", { value: true });
var claude_code_1 = require("@anthropic-ai/claude-code");
var messages = [];
try {
    for (var _d = true, _e = __asyncValues((0, claude_code_1.query)({
        prompt: "login",
        abortController: new AbortController(),
        options: {
            appendSystemPrompt: "You're task is to determine a good branch name for the users request. YOU MUST respond with ONLY a JSON object with the format  {branchName: []}",
            maxTurns: 1,
            allowedTools: [],
            model: "claude-sonnet-4"
        },
    })), _f; _f = await _e.next(), _a = _f.done, !_a; _d = true) {
        _c = _f.value;
        _d = false;
        var message = _c;
        messages.push(message);
    }
}
catch (e_1_1) { e_1 = { error: e_1_1 }; }
finally {
    try {
        if (!_d && !_a && (_b = _e.return)) await _b.call(_e);
    }
    finally { if (e_1) throw e_1.error; }
}
console.log(JSON.stringify(messages, null, 2));
