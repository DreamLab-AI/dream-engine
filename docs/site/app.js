/* ==========================================================================
   Dream Engine — shared front-end behaviour
   Everything is feature-detected and fails soft: with JS off or an API
   missing, all content stays readable and every command stays selectable.
   ========================================================================== */

(function () {
  "use strict";

  var root = document.documentElement;
  root.classList.add("js");

  var motionQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)")
      : null;

  function reducedMotion() {
    return !!(motionQuery && motionQuery.matches);
  }

  function onMotionChange(handler) {
    if (!motionQuery) return;
    if (typeof motionQuery.addEventListener === "function") {
      motionQuery.addEventListener("change", handler);
    } else if (typeof motionQuery.addListener === "function") {
      motionQuery.addListener(handler);
    }
  }

  var hasIO = typeof window.IntersectionObserver === "function";
  var raf =
    window.requestAnimationFrame ||
    function (fn) {
      return window.setTimeout(fn, 16);
    };

  /* ------------------------------------------------------------------------
     1. Starfield — drawn to canvas, no image assets
     ---------------------------------------------------------------------- */

  function initStarfield() {
    var canvas = document.querySelector(".sky__stars");
    if (!canvas || typeof canvas.getContext !== "function") return;

    var ctx = canvas.getContext("2d");
    if (!ctx) return;

    var stars = [];
    var dpr = Math.min(window.devicePixelRatio || 1, 2);
    var w = 0;
    var h = 0;

    function seed() {
      w = canvas.clientWidth || window.innerWidth;
      h = canvas.clientHeight || window.innerHeight;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      var count = Math.min(280, Math.round((w * h) / 5200));
      stars = [];
      for (var i = 0; i < count; i++) {
        var big = Math.random() > 0.93;
        stars.push({
          x: Math.random() * w,
          y: Math.random() * h,
          r: big ? 1.1 + Math.random() * 0.9 : 0.35 + Math.random() * 0.7,
          a: 0.25 + Math.random() * 0.55,
          phase: Math.random() * Math.PI * 2,
          speed: 0.4 + Math.random() * 0.9,
          hue: Math.random() > 0.82 ? "167, 232, 255" : "225, 230, 255"
        });
      }
    }

    function draw(t) {
      ctx.clearRect(0, 0, w, h);
      var still = reducedMotion();
      for (var i = 0; i < stars.length; i++) {
        var s = stars[i];
        var alpha = still
          ? s.a
          : s.a * (0.62 + 0.38 * Math.sin(t / 1400 * s.speed + s.phase));
        ctx.beginPath();
        ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
        ctx.fillStyle = "rgba(" + s.hue + ", " + alpha.toFixed(3) + ")";
        ctx.fill();
      }
    }

    var running = false;

    function loop(t) {
      draw(t || 0);
      if (running) raf(loop);
    }

    function start() {
      if (reducedMotion()) {
        running = false;
        draw(0);
        return;
      }
      if (running) return;
      running = true;
      raf(loop);
    }

    var resizeTimer;
    window.addEventListener(
      "resize",
      function () {
        window.clearTimeout(resizeTimer);
        resizeTimer = window.setTimeout(function () {
          seed();
          draw(0);
        }, 180);
      },
      { passive: true }
    );

    onMotionChange(function () {
      running = false;
      start();
    });

    seed();
    start();
  }

  /* ------------------------------------------------------------------------
     2. Layered parallax — background nebulae drift slower than the content
     ---------------------------------------------------------------------- */

  function initParallax() {
    var layers = [].slice.call(document.querySelectorAll("[data-parallax]"));
    if (!layers.length) return;

    var ticking = false;

    function clear() {
      for (var i = 0; i < layers.length; i++) {
        layers[i].style.transform = "";
      }
    }

    function apply() {
      ticking = false;
      if (reducedMotion()) return;
      var y = window.pageYOffset || root.scrollTop || 0;
      for (var i = 0; i < layers.length; i++) {
        var el = layers[i];
        var factor = parseFloat(el.getAttribute("data-parallax")) || 0;
        el.style.transform =
          "translate3d(0, " + (-y * factor).toFixed(2) + "px, 0)";
      }
    }

    function onScroll() {
      if (ticking) return;
      ticking = true;
      raf(apply);
    }

    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll, { passive: true });

    onMotionChange(function () {
      if (reducedMotion()) clear();
      else onScroll();
    });

    if (reducedMotion()) clear();
    else apply();
  }

  /* ------------------------------------------------------------------------
     3. Sticky header state
     ---------------------------------------------------------------------- */

  function initHeader() {
    var header = document.querySelector(".site-header");
    if (!header) return;

    function update() {
      var y = window.pageYOffset || root.scrollTop || 0;
      header.classList.toggle("is-stuck", y > 12);
    }

    window.addEventListener("scroll", update, { passive: true });
    update();
  }

  /* ------------------------------------------------------------------------
     4. Reveal on scroll (purely decorative — content is visible without it)
     ---------------------------------------------------------------------- */

  function initReveal() {
    var items = [].slice.call(document.querySelectorAll(".reveal"));
    if (!items.length) return;

    if (!hasIO || reducedMotion()) {
      items.forEach(function (el) {
        el.classList.add("is-in");
      });
      return;
    }

    var io = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (entry.isIntersecting) {
            entry.target.classList.add("is-in");
            io.unobserve(entry.target);
          }
        });
      },
      { rootMargin: "0px 0px -12% 0px", threshold: 0.08 }
    );

    items.forEach(function (el) {
      io.observe(el);
    });
  }

  /* ------------------------------------------------------------------------
     5. Copy-to-clipboard, with a select-the-text fallback
     ---------------------------------------------------------------------- */

  function announce(message) {
    var live = document.getElementById("copy-status");
    if (live) live.textContent = message;
  }

  function legacyCopy(text) {
    var ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-1000px";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    var ok = false;
    try {
      ta.select();
      ok = document.execCommand("copy");
    } catch (err) {
      ok = false;
    }
    document.body.removeChild(ta);
    return ok;
  }

  function selectSource(btn) {
    var block = btn.closest ? btn.closest(".code, .cmd") : null;
    var target = block && (block.querySelector("code") || block.querySelector(".cmd__text"));
    if (!target || !window.getSelection || !document.createRange) return;
    var range = document.createRange();
    range.selectNodeContents(target);
    var sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function sourceText(btn) {
    if (btn.hasAttribute("data-copy")) return btn.getAttribute("data-copy");
    var block = btn.closest ? btn.closest(".code, .cmd") : null;
    var code = block && block.querySelector("code");
    return code ? code.textContent.replace(/\s+$/, "") : "";
  }

  function flash(btn, label) {
    var slot = btn.querySelector(".copy-label");
    if (!slot) return;
    var original = btn.getAttribute("data-label") || slot.textContent;
    btn.setAttribute("data-label", original);
    slot.textContent = label;
    window.setTimeout(function () {
      slot.textContent = original;
    }, 1800);
  }

  function initCopy() {
    var buttons = [].slice.call(document.querySelectorAll("[data-copy-btn]"));
    if (!buttons.length) return;

    buttons.forEach(function (btn) {
      btn.addEventListener("click", function () {
        var text = sourceText(btn);
        if (!text) return;

        function done(ok) {
          if (ok) {
            flash(btn, "Copied");
            announce("Copied to clipboard: " + text);
          } else {
            flash(btn, "Select + copy");
            selectSource(btn);
            announce("Clipboard unavailable — the command is selected, press Ctrl or Cmd + C.");
          }
        }

        if (navigator.clipboard && window.isSecureContext) {
          navigator.clipboard.writeText(text).then(
            function () {
              done(true);
            },
            function () {
              done(legacyCopy(text));
            }
          );
        } else {
          done(legacyCopy(text));
        }
      });
    });
  }

  /* ------------------------------------------------------------------------
     6. Scrollytelling pipeline — step observer drives the orbit visual
     ---------------------------------------------------------------------- */

  function initScrolly() {
    var scrolly = document.querySelector("[data-scrolly]");
    if (!scrolly) return;

    var steps = [].slice.call(scrolly.querySelectorAll(".stage"));
    var nodes = [].slice.call(scrolly.querySelectorAll(".orbit__node"));
    var arc = scrolly.querySelector(".orbit__arc");
    var centerNum = scrolly.querySelector("[data-orbit-num]");
    var centerLabel = scrolly.querySelector("[data-orbit-label]");
    var centerSub = scrolly.querySelector("[data-orbit-sub]");
    var readout = scrolly.querySelector("[data-orbit-readout]");
    if (!steps.length) return;

    var ARC_LENGTH = 741.42; /* 2 * PI * r, r = 118 */
    var current = -1;

    function activate(index) {
      if (index === current || index < 0 || index >= steps.length) return;
      current = index;

      steps.forEach(function (step, i) {
        step.classList.toggle("is-active", i === index);
      });

      nodes.forEach(function (node, i) {
        node.classList.toggle("is-active", i === index);
        node.classList.toggle("is-past", i < index);
      });

      var step = steps[index];
      if (centerNum) {
        centerNum.textContent =
          "STAGE " + String(index + 1).padStart(2, "0");
      }
      if (centerLabel) {
        centerLabel.textContent = step.getAttribute("data-short") || "";
      }
      if (centerSub) {
        centerSub.textContent = step.getAttribute("data-out") || "";
      }
      if (readout) {
        readout.textContent =
          String(index + 1).padStart(2, "0") + " / " + String(steps.length).padStart(2, "0");
      }
      if (arc) {
        var progress = (index + 1) / steps.length;
        arc.style.strokeDasharray = ARC_LENGTH;
        arc.style.strokeDashoffset = (ARC_LENGTH * (1 - progress)).toFixed(2);
      }
    }

    if (!hasIO) {
      steps.forEach(function (step) {
        step.classList.add("is-active");
      });
      activate(0);
      return;
    }

    var io = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          var index = steps.indexOf(entry.target);
          if (index > -1) activate(index);
        });
      },
      { rootMargin: "-45% 0px -45% 0px", threshold: 0 }
    );

    steps.forEach(function (step) {
      io.observe(step);
    });

    activate(0);
  }

  /* ------------------------------------------------------------------------
     Boot
     ---------------------------------------------------------------------- */

  function boot() {
    try {
      initStarfield();
    } catch (err) {
      /* backdrop is decorative — never block the page */
    }
    initParallax();
    initHeader();
    initReveal();
    initCopy();
    initScrolly();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();
