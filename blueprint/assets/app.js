    (() => {
      const sourceDetails = () => document.querySelectorAll(".full-md-item");
      const sections = [...document.querySelectorAll("main section[id]")];
      const pageItems = [...document.querySelectorAll("[data-page]")];
      const pageLinks = [...document.querySelectorAll("[data-page-link]")];
      const sideLinks = [...document.querySelectorAll(".side a")];
      const tocNav = document.querySelector(".toc nav");
      const navLinks = () => [...document.querySelectorAll(".side a:not([hidden]), .toc a")];
      const searchDialog = document.getElementById("search-dialog");
      const searchInput = document.getElementById("search-input");
      const searchResults = document.getElementById("search-results");
      const progressBar = document.querySelector(".reading-progress span");
      const pageTitleText = document.querySelector(".page-titlebar span:last-child");
      const pageSections = {
        overview: ["vision", "product-shape", "users", "product-workflows", "north-star", "positioning", "product-boundaries"],
        architecture: ["principles", "architecture", "capabilities", "providers", "bridge", "platform"],
        marketplace: ["repository"],
        roadmap: ["development-route", "roadmap", "phase-one", "decisions"],
        appendix: ["source-template", "sources"]
      };
      const sectionPage = Object.fromEntries(
        Object.entries(pageSections).flatMap(([page, ids]) => ids.map((id) => [id, page]))
      );
      const pageTitles = {
        overview: "PingoGate 开发蓝图 · 产品",
        architecture: "PingoGate 开发蓝图 · 架构",
        marketplace: "PingoGate 开发蓝图 · 分发",
        roadmap: "PingoGate 开发蓝图 · 路线",
        appendix: "PingoGate 开发蓝图 · 附录"
      };
      let currentPage = "overview";

      const searchableSections = sections.map((section) => {
        const heading = section.querySelector("h1, h2")?.textContent?.trim() || section.id;
        const note = section.querySelector(".section-note, .lead, p")?.textContent?.trim() || "";
        return {
          id: section.id,
          heading,
          note,
          haystack: `${heading} ${note} ${section.textContent}`.toLowerCase()
        };
      });
      let fuseSearch = null;

      const escapeHtml = (value) => value.replace(/[&<>"']/g, (char) => ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        "\"": "&quot;",
        "'": "&#39;"
      })[char]);

      const currentPageFromHash = () => {
        const raw = location.hash.replace(/^#/, "");
        if (raw.startsWith("page-")) return raw.slice(5);
        return sectionPage[raw] || "overview";
      };

      const setActiveNav = (id) => {
        navLinks().forEach((link) => {
          link.classList.toggle("active", link.getAttribute("href") === `#${id}`);
        });
      };

      const renderPageToc = (page) => {
        if (!tocNav) return;
        const ids = pageSections[page] || pageSections.overview;
        tocNav.innerHTML = ids.map((id, index) => {
          const section = document.getElementById(id);
          const label = section?.querySelector("h1, h2")?.textContent?.trim() || id;
          return `<a ${index === 0 ? 'class="active"' : ""} href="#${id}">${escapeHtml(label)}</a>`;
        }).join("");
      };

      const setPage = (page, targetId = "") => {
        const nextPage = pageSections[page] ? page : "overview";
        currentPage = nextPage;
        if (pageTitleText) pageTitleText.textContent = pageTitles[nextPage] || "PingoGate 开发蓝图";
        pageItems.forEach((item) => {
          item.hidden = item.dataset.page !== nextPage;
        });
        pageLinks.forEach((link) => {
          link.classList.toggle("active", link.dataset.pageLink === nextPage);
        });
        sideLinks.forEach((link) => {
          const id = link.getAttribute("href")?.slice(1);
          link.hidden = !id || sectionPage[id] !== nextPage;
        });
        document.querySelectorAll(".side .nav-group").forEach((group) => {
          group.hidden = !group.querySelector("a:not([hidden])");
        });
        renderPageToc(nextPage);
        const firstId = targetId || pageSections[nextPage][0];
        setActiveNav(firstId);
        if (targetId) {
          requestAnimationFrame(() => {
            document.getElementById(targetId)?.scrollIntoView({ block: "start" });
          });
        }
        updateProgress();
      };

      const updateProgress = () => {
        const max = document.documentElement.scrollHeight - window.innerHeight;
        const percent = max > 0 ? Math.min(100, Math.max(0, (window.scrollY / max) * 100)) : 0;
        if (progressBar) progressBar.style.width = `${percent}%`;
      };

      const renderSearch = (query = "") => {
        const value = query.trim().toLowerCase();
        const scopedSections = searchableSections.filter((item) => sectionPage[item.id] === currentPage);
        const matches = value
          ? (fuseSearch
              ? fuseSearch.search(value).map((result) => result.item).filter((item) => sectionPage[item.id] === currentPage).slice(0, 12)
              : scopedSections.filter((item) => item.haystack.includes(value)).slice(0, 12))
          : scopedSections.slice(0, 8);

        if (!searchResults) return;
        searchResults.innerHTML = matches.length
          ? matches.map((item) => `
              <a class="search-result" href="#${item.id}" role="option" data-search-hit data-target-page="${sectionPage[item.id] || "overview"}">
                <strong>${escapeHtml(item.heading)}</strong>
                <span>${escapeHtml(item.note || "打开这个章节")}</span>
              </a>
            `).join("")
          : '<div class="search-empty">没有找到匹配内容</div>';
      };

      const openSearch = () => {
        if (!searchDialog) return;
        renderSearch(searchInput?.value || "");
        searchDialog.showModal();
        requestAnimationFrame(() => searchInput?.focus());
      };

      document.querySelectorAll("[data-open-search]").forEach((button) => {
        button.addEventListener("click", openSearch);
      });

      pageLinks.forEach((link) => {
        link.addEventListener("click", (event) => {
          event.preventDefault();
          const page = link.dataset.pageLink;
          history.pushState(null, "", `#page-${page}`);
          setPage(page);
          window.scrollTo({ top: 0, behavior: "smooth" });
        });
      });

      document.addEventListener("click", (event) => {
        const link = event.target.closest('a[href^="#"]:not([data-page-link])');
        if (!link) return;
        const id = link.getAttribute("href")?.slice(1);
        const targetPage = sectionPage[id];
        if (!id || !targetPage) return;
        event.preventDefault();
        history.pushState(null, "", `#${id}`);
        setPage(targetPage, id);
      });

      document.addEventListener("keydown", (event) => {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
          event.preventDefault();
          openSearch();
        }
        if (event.key === "Escape" && searchDialog?.open) {
          searchDialog.close();
        }
      });

      searchInput?.addEventListener("input", () => renderSearch(searchInput.value));

      searchResults?.addEventListener("click", (event) => {
        const hit = event.target.closest("[data-search-hit]");
        if (hit && searchDialog?.open) {
          setPage(hit.dataset.targetPage, hit.getAttribute("href")?.slice(1));
          searchDialog.close();
        }
      });

      const observer = new IntersectionObserver((entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
        if (visible?.target?.id) {
          setActiveNav(visible.target.id);
        }
      }, {
        rootMargin: "-20% 0px -65% 0px",
        threshold: [0.1, 0.25, 0.5]
      });

      sections.forEach((section) => observer.observe(section));
      setPage(currentPageFromHash(), location.hash.replace(/^#/, "").startsWith("page-") ? "" : location.hash.slice(1));
      window.addEventListener("load", () => {
        if (window.lucide?.createIcons) {
          window.lucide.createIcons();
        }
        if (window.Fuse) {
          fuseSearch = new window.Fuse(searchableSections, {
            keys: ["heading", "note", "haystack"],
            threshold: 0.32,
            ignoreLocation: true
          });
        }
      });
      updateProgress();
      window.addEventListener("scroll", updateProgress, { passive: true });
      window.addEventListener("hashchange", () => {
        const id = location.hash.slice(1);
        if (id.startsWith("page-")) {
          setPage(id.slice(5));
        } else {
          setPage(sectionPage[id] || "overview", id);
        }
      });

      document.querySelectorAll("[data-source-action]").forEach((button) => {
        button.addEventListener("click", () => {
          const shouldOpen = button.dataset.sourceAction === "open";
          sourceDetails().forEach((item) => {
            item.open = shouldOpen;
          });
        });
      });

      document.querySelector("[data-copy-markdown]")?.addEventListener("click", async (event) => {
        const markdown = [...document.querySelectorAll(".full-md-item pre")]
          .map((pre) => pre.textContent.trim())
          .filter(Boolean)
          .join("\n\n");
        try {
          await navigator.clipboard.writeText(markdown);
          event.currentTarget.textContent = "已复制";
          setTimeout(() => {
            event.currentTarget.textContent = "复制 Markdown";
          }, 1400);
        } catch {
          event.currentTarget.textContent = "复制失败";
          setTimeout(() => {
            event.currentTarget.textContent = "复制 Markdown";
          }, 1400);
        }
      });

      window.addEventListener("beforeprint", () => {
        sourceDetails().forEach((item) => {
          item.dataset.printOpen = item.open ? "true" : "false";
          item.open = true;
        });
      });

      window.addEventListener("afterprint", () => {
        sourceDetails().forEach((item) => {
          item.open = item.dataset.printOpen === "true";
          delete item.dataset.printOpen;
        });
      });

      // Off-canvas hamburger drawer for narrow viewports (<=980px).
      // Mirrors the behaviour of the Coding showcase: button toggles, scrim closes,
      // ESC closes, in-drawer link click closes, and switching back to desktop resets state.
      const menuToggle = document.getElementById("menu-toggle");
      const sideDrawer = document.getElementById("docs-side");
      const scrim = document.getElementById("menu-scrim");
      const desktopQuery = window.matchMedia("(min-width: 981px)");

      const setDrawer = (open) => {
        if (!menuToggle || !sideDrawer || !scrim) return;
        sideDrawer.classList.toggle("open", open);
        scrim.classList.toggle("open", open);
        scrim.hidden = !open;
        menuToggle.setAttribute("aria-expanded", String(open));
        document.body.style.overflow = open ? "hidden" : "";
      };

      if (menuToggle && sideDrawer && scrim) {
        menuToggle.addEventListener("click", () => {
          setDrawer(!sideDrawer.classList.contains("open"));
        });
        scrim.addEventListener("click", () => setDrawer(false));
        document.addEventListener("keydown", (e) => {
          if (e.key === "Escape" && sideDrawer.classList.contains("open")) {
            setDrawer(false);
          }
        });
        // Any anchor link inside the drawer closes it after navigation.
        sideDrawer.addEventListener("click", (e) => {
          const link = e.target.closest("a[href]");
          if (link) setDrawer(false);
        });
        // When the viewport grows back to desktop, drop any drawer state so the
        // sticky sidebar layout resumes cleanly.
        desktopQuery.addEventListener("change", (e) => {
          if (e.matches) setDrawer(false);
        });
      }
    })();
