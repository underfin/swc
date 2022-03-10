import * as swcHelpers from "@swc/helpers";
var _f = new WeakMap(), _g = new WeakMap(), _h = new WeakMap(), _i = new WeakMap();
// @strict: true
// @target:es2015
// @declaration: true
// Properties with non-undefined types require initialization
var C1 = function C1() {
    "use strict";
    swcHelpers.classCallCheck(this, C1);
    swcHelpers.classPrivateFieldInit(this, _f, {
        writable: true,
        value: void 0 //Error
    });
    swcHelpers.classPrivateFieldInit(this, _g, {
        writable: true,
        value: void 0
    });
    swcHelpers.classPrivateFieldInit(this, _h, {
        writable: true,
        value: void 0 //Error
    });
    swcHelpers.classPrivateFieldInit(this, _i, {
        writable: true,
        value: void 0
    });
};
// No strict initialization checks for static members
var C3 = function C3() {
    "use strict";
    swcHelpers.classCallCheck(this, C3);
};
var _d = new WeakMap(), _e = new WeakMap(), _f1 = new WeakMap();
// Initializer satisfies strict initialization check
var C4 = function C4() {
    "use strict";
    swcHelpers.classCallCheck(this, C4);
    this.a = 0;
    this.b = 0;
    this.c = "abc";
    swcHelpers.classPrivateFieldInit(this, _d, {
        writable: true,
        value: 0
    });
    swcHelpers.classPrivateFieldInit(this, _e, {
        writable: true,
        value: 0
    });
    swcHelpers.classPrivateFieldInit(this, _f1, {
        writable: true,
        value: "abc"
    });
};
var _b = new WeakMap();
// Assignment in constructor satisfies strict initialization check
var C5 = function C5() {
    "use strict";
    swcHelpers.classCallCheck(this, C5);
    swcHelpers.classPrivateFieldInit(this, _b, {
        writable: true,
        value: void 0
    });
    this.a = 0;
    swcHelpers.classPrivateFieldSet(this, _b, 0);
};
var _b1 = new WeakMap();
// All code paths must contain assignment
var C6 = function C6(cond) {
    "use strict";
    swcHelpers.classCallCheck(this, C6);
    swcHelpers.classPrivateFieldInit(this, _b1, {
        writable: true,
        value: void 0
    });
    if (cond) {
        return;
    }
    this.a = 0;
    swcHelpers.classPrivateFieldSet(this, _b1, 0);
};
var _b2 = new WeakMap();
var C7 = function C7(cond) {
    "use strict";
    swcHelpers.classCallCheck(this, C7);
    swcHelpers.classPrivateFieldInit(this, _b2, {
        writable: true,
        value: void 0
    });
    if (cond) {
        this.a = 1;
        swcHelpers.classPrivateFieldSet(this, _b2, 1);
        return;
    }
    this.a = 0;
    swcHelpers.classPrivateFieldSet(this, _b2, 1);
};
// Properties with string literal names aren't checked
var C8 = function C8() {
    "use strict";
    swcHelpers.classCallCheck(this, C8);
};
// No strict initialization checks for abstract members
var C9 = function C9() {
    "use strict";
    swcHelpers.classCallCheck(this, C9);
};
var _d1 = new WeakMap();
// Properties with non-undefined types must be assigned before they can be accessed
// within their constructor
var C10 = function C10() {
    "use strict";
    swcHelpers.classCallCheck(this, C10);
    swcHelpers.classPrivateFieldInit(this, _d1, {
        writable: true,
        value: void 0
    });
    var x = this.a; // Error
    this.a = this.b; // Error
    this.b = swcHelpers.classPrivateFieldGet(this, _d1 //Error
    );
    this.b = x;
    swcHelpers.classPrivateFieldSet(this, _d1, x);
    var y = this.c;
};
var _b3 = new WeakMap();
var C11 = function C11() {
    "use strict";
    swcHelpers.classCallCheck(this, C11);
    swcHelpers.classPrivateFieldInit(this, _b3, {
        writable: true,
        value: void 0
    });
    this.a = someValue();
    swcHelpers.classPrivateFieldSet(this, _b3, someValue());
};
