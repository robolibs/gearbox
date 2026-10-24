# The Real Science of Growing Plants on Mars (state as of October 2026)

Scope note: the notes favour numbers a simulator can use. Most foundational data are old: the NASA Biomass Production Chamber (1987–2000), the Mars Greenhouses workshop (1999), Wageningen (2013–2019) and EDEN ISS (2018). Newer work (2021–2026) is mostly regolith-simulant amendment studies. No plant has ever been grown on Mars itself, or in real Martian regolith. Every "Mars soil" result below uses a terrestrial simulant.

---

## 1. Mars surface conditions relevant to plants

### Takeaway
Outside, Mars is lethal to plants. Pressure is about 0.6 kPa, almost entirely CO2, mean temperature is about −59 °C, and there is essentially no liquid water. Unfiltered UV is about 1000× Earth's in DNA-damage terms. The light, though, is usable: top-of-atmosphere irradiance is about 586 W/m² (about 43% of Earth's), and clear-sky surface light is roughly 44–60% of Earth's. Dust storms can cut direct light almost completely for weeks. A greenhouse must provide pressure, heat, water, UV filtering and radiation shielding. Natural light is the one input it can partly take from outside.

### Cited Findings
**Atmosphere and pressure**
- Mean surface pressure is 6.36 mbar (0.636 kPa) at mean radius. It varies seasonally from 4.0 to 8.7 mbar. Surface density is about 0.016 kg/m³ and the scale height is 11.0 km. — [NASA NSSDC Mars Fact Sheet](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- Composition by volume: CO2 95.1%, N2 2.59%, Ar 1.94%, O2 0.16%, CO 0.06%. — [NASA NSSDC Mars Fact Sheet](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- Inference from the above: ambient pCO2 is about 0.6 kPa, which is about 15× Earth's 0.04 kPa. Ambient pO2 is about 0.001 kPa. A pressurised greenhouse could take its CO2 directly from outside air (see BVAD's note that CO2 "is readily extracted from the atmosphere, which is 95% carbon dioxide, though at a low pressure" — [NASA BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)).

**Temperature and diurnal cycle**
- Mean surface temperature is about 214 K (−59 °C). The diurnal range at the Viking 1 site was 184–242 K (−89 to −31 °C). — [NASA NSSDC](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- Extremes run from about +20 °C (equator, summer noon) down to about −153 °C (poles). At the equator at noon it can be about 24 °C at foot level and about 0 °C at head level. — [NASA Science, Mars Facts](https://science.nasa.gov/mars/facts/)
- The 2018 global dust storm (MY34) shrank the diurnal range at Gale crater by 35 K (ground) and 56 K (air), as measured by Curiosity's REMS. — [Effects of the MY34/2018 Global Dust Storm, MSL REMS](https://deepblue.lib.umich.edu/items/fd84b4df-da4c-4348-b590-89eebd6af25f)

**Sol, year, seasons, gravity**
- Length of day is 24.6597 h. The sidereal year is 686.98 Earth days. Obliquity is 25.19°. Mean surface gravity is 3.73 m/s² (about 0.38 g). — [NASA NSSDC](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- The year is 669.6 sols. Season lengths (northern hemisphere): spring 194 sols, summer 178 sols, autumn 142, winter 154 sols. — [NASA Science, Mars Facts](https://science.nasa.gov/mars/facts/)

**Solar irradiance and light for photosynthesis**
- Top-of-atmosphere solar irradiance at Mars is 586.2 W/m² (mean). — [NASA NSSDC](https://nssdc.gsfc.nasa.gov/planetary/factsheet/marsfact.html)
- Rygalov, Bucklin, Fowler and Wheeler (NASA, 2000) give 589 ± 142 W/m² for the Mars solar constant; the spread is from orbital eccentricity. They give an "optimal" plant irradiance of 80–180 W/m² PAR and an optimal temperature of 20–27 °C. — [NTRS 20050182979](https://ntrs.nasa.gov/citations/20050182979)
- An education source puts direct irradiance at about 590 W/m² with the sun overhead, against about 1000 W/m² on Earth, and says the spectral shapes are essentially the same. This is not a peer-reviewed source. — [First the Seed Foundation, Sunlight on Mars](https://www.firsttheseedfoundation.org/wp-content/uploads/2016/08/Sunlight-on-Mars.pdf)
- Wamelink: Mars receives about 60% of Earth's light, so plants would grow at roughly 60% of Earth's rate (press quote, 2017). — [ScienceAlert](https://www.sciencealert.com/earthworms-can-reproduce-in-fertilised-mars-soil-simulant)
- Villanova greenhouse trials used light at about 44% of Earth level to mimic Mars. — [Eos, 2018](https://eos.org/articles/tests-indicate-which-edible-plants-could-thrive-on-mars)
- Appelbaum & Flood (NASA Lewis, 1989–1994) is the standard model for Mars global, beam and diffuse insolation. It uses Viking-measured optical depth, Beer's law for the beam, and multiple-scattering tables for the diffuse part. Insolation depends on latitude, season, optical depth and surface tracking. — [NTRS 19910005804](https://ntrs.nasa.gov/citations/19910005804); [NTRS 19950004977](https://ntrs.nasa.gov/citations/19950004977). An R package (`marsrad`) implements these equations, including daily horizontal insolation `H_h`. — [CRAN marsrad](https://cran.r-project.org/web/packages/marsrad/readme/README.html)
- A 2005 SAE paper combined spectral irradiance and multi-year opacity models to estimate global surface PAR for greenhouse siting. Only the abstract was found. — [SAE 2005-01-2813](https://saemobilus.sae.org/papers/global-estimates-photosynthetically-active-radiation-mars-surface-2005-01-2813)

**Dust optical depth (tau)**
- Clear-day visible optical depth ranges from 0.1 to 1.0. A localised dust storm gives about 2.0, a severe storm about 6.0. — [Cockell et al. 2000, Icarus 146:343](https://faculty.washington.edu/dcatling/papers_mine/Cockell_Catling2000.pdf)
- Opportunity measured tau ≈ 10.8 on 10 June 2018, the highest ever recorded at the surface. The previous mission high was 5–5.5 (2007). Typical tau at Endeavour crater was about 0.5. — [Planetary Society](https://www.planetary.org/explore/space-topics/space-missions/mer-updates/2018/06-mer-update-opportunity-dust-storm-sleep.html); [JPL](https://www.jpl.nasa.gov/news/opportunity-hunkers-down-during-dust-storm/)
- Tau probably had to fall below about 2.0 before Opportunity's panels could recharge its batteries. — [Planetary Society](https://www.planetary.org/explore/space-topics/space-missions/mer-updates/2018/06-mer-update-opportunity-dust-storm-sleep.html). The images from June 2018 were its last. — [PIA22930: Last Images Opportunity Took](https://pds-rings.seti.org/press_releases/pages/PIA22xxx/PIA22930.html)
- Dust storms can cover much of the planet, and the dust can take months to settle. — [NASA Science](https://science.nasa.gov/mars/facts/)

**UV**
- Present-day Mars at the equator, vernal equinox, sun at zenith, clear sky (tau = 0.1), from Cockell et al. 2000 Table I. UVC+UVB (200–315 nm) is about 13.2 W/m² instantaneous and 361 kJ/m² daily fluence; Earth is 1.86 W/m² and 39 kJ/m². UVA (315–400 nm) is 41.5 W/m² against Earth's 52.8 W/m². — [Cockell et al. 2000](https://faculty.washington.edu/dcatling/papers_mine/Cockell_Catling2000.pdf)
- Earth's DNA-damage-weighted dose is about 0.1% of present-day Mars's (a factor of about 1000), and plant (photosystem) damage about 0.29% (a factor of about 345). Wavelengths below about 200 nm do not reach the surface because CO2 absorbs them. — [Cockell et al. 2000](https://faculty.washington.edu/dcatling/papers_mine/Cockell_Catling2000.pdf)
- UV-activated perchlorate killed *Bacillus subtilis* within minutes under Mars-like conditions; iron oxides and H2O2 made it worse (Wadsworth & Cockell 2017). — [Space.com](https://space.com/37402-mars-life-soil-toxic-perchlorates-radiation.html)

**Ionising radiation**
- Curiosity RAD measured a mean surface GCR dose-equivalent of 0.67 mSv/day (Aug 2012 – Jun 2013), against about 1.8 mSv/day during cruise (Hassler et al., *Science*, 2013/2014). — [SwRI press release](https://www.swri.org/press-release/swri-scientists-publish-first-radiation-measurements-surface-mars)
- A DLR abstract gives 0.64 ± 0.12 mSv/day. — [DLR elib 106090](https://elib.dlr.de/106090)

**Water**
- Curiosity's SAM found about 2 wt% water in Rocknest surface soil, released by heating to 835 °C. That is about 1 L per cubic foot (0.03 m³) of dirt (Leshin et al. 2013). Its isotope ratio suggests the soil absorbs water from the air. — [NASA](https://www.nasa.gov/missions/mars-science-laboratory/curiosity-rover/curiositys-sam-instrument-finds-water-and-more-in-surface-sample/); [Space.com](https://space.com/22949-mars-water-discovery-curiosity-rover.html)
- Gale crater is very dry and never reached saturation in the first 100 sols. Relative humidity peaks just before sunrise, at up to about 29% before the 2018 storm and under 5% during it. — [REMS first 100 sols](https://deepblue.lib.umich.edu/items/731df323-c5bb-46d1-8cfc-e7be056a0d9a); [REMS MY34](https://deepblue.lib.umich.edu/items/fd84b4df-da4c-4348-b590-89eebd6af25f)
- Phoenix (68°N) sampled icy regolith containing about 0.6% perchlorate. Perchlorate brines mean ice deposits may hold the same or higher concentrations. — [Rothschild et al., LPSC 2025 #2713](https://epizodyspace.ru/bibl/inostr-yazyki/Lunar_and_Planetary_Science/2025/Rothschild_et_al_Detoxifying_Mars_56th_LPSC_no_2713_(2025).pdf)

### Inferences
- Rough PAR estimate (my own, not from a source): PAR is about 45–50% of broadband shortwave, and broadband sunlight is about 4.57 µmol per joule of PAR. On that basis, 590 W/m² overhead and clear gives roughly 260–290 W/m² PAR, about 1200–1300 µmol/m²/s at the top of the atmosphere. At tau 0.3–0.5, with diffuse light partly making up the loss, clear-noon surface PPFD is plausibly about 900–1100 µmol/m²/s, against about 2000 on Earth. A half-sine over about 12.3 h of daylight then gives a clear-sky outdoor DLI of roughly 25–30 mol/m²/sol at the equinox, before greenhouse glazing losses. Treat these as order-of-magnitude values. The Appelbaum & Flood tables (or `marsrad`) are the right way to compute them properly.
- Rygalov's 80–180 W/m² PAR target converts to about 370–830 µmol/m²/s. That is inside the 200–1000 µmol/m²/s range that NASA's crop models are valid for (see section 2).
- For a simulator: model light as top-of-atmosphere irradiance scaled by distance (±24%) × cos(zenith) × exp(−tau/cos z) for the beam, plus a diffuse term that grows as a share of the total when tau is high. During a global storm (tau 5–11) the direct beam is almost gone. Natural-light greenhouses then need LED backup or the crops stall. This matches Opportunity's power failure.
- A sol is 24.66 h, so a 12/12 photoperiod set in sol-hours runs about 2.7% longer than on Earth.

### Gaps
- I found no peer-reviewed table of surface PPFD (µmol/m²/s) against tau and solar zenith angle. The SAE 2005 paper's results were not available.
- I found no data on how plants respond to a 24.66 h day or to 0.38 g. All gravity data I found were from microgravity (ISS). I did not research 0.38 g partial-gravity centrifuge studies.
- I found no single primary source for typical multi-sol temperature extremes at specific landing sites beyond Viking 1 and the REMS storm comparison.

---

## 2. What plants need inside a Mars greenhouse or habitat

### Takeaway
Plants tolerate far lower total pressure than humans. Normal function holds down to about 10 kPa, provided O2 stays at roughly 5–7 kPa or more and CO2 is enriched. Below about 25–33 kPa, expect higher water loss and drought-like stress responses. Proposed Mars greenhouse designs sit at 10–20 kPa. NASA's design values are pCO2 0.10–0.20 kPa (1000–2000 ppm at 1 atm), RH about 75%, and air at 20–28 °C depending on crop. Lettuce needs a DLI of about 17 mol/m²/d (16 h). Potato and soybean need about 28 (12 h). Wheat runs at up to about 115 mol/m²/d with a 20–24 h photoperiod. Light is typically red+blue LED, with some green or white added.

### Cited Findings
**Total pressure (hypobaric studies)**
- Corey, Fowler & Wheeler (NASA KSC, 2000) say normal plant function is likely limited to about 10 kPa or slightly lower. In lettuce at 10 kPa, water loss was 6.1× the rate at 101 kPa. — [NTRS 20050182975](https://ntrs.nasa.gov/citations/20050182975)
- Wheeler et al. (2000) give a theoretical minimum of about 9 kPa, about 1/10 atm. Lower pressure also weakens temperature and humidity control, which can cancel out the mass savings. — [IAC-11 abstract](https://iafastro.directory/iac/archive/browse/IAC-11/A1/6/9532/)
- Bucklin et al. (2004, deployable Mars dome) say planned Mars greenhouses would run at 0.1–0.2 atm (about 10–20 kPa). They report preliminary lettuce growth tests at 25 kPa. — [AGRIS](https://agris.fao.org/search/en/records/65df75dd4c5aef494fe2c2dd)
- With current Mars-buildable materials, a greenhouse is feasible only if internal pressure stays below about 1/16 atm (about 6 kPa). This is an engineering constraint, not a plant one. — [SpaceNews](https://spacenews.com/?p=40051)
- Radish: growth did not differ significantly between 98, 66, 33 and 10 kPa total pressure as long as pO2 was at least 7 kPa. Growth fell at pO2 2 kPa whatever the total pressure. So O2 partial pressure, not total pressure, is the limiting factor. — [CORDIS radish report, 2012](https://www.cordis.europa.eu/docs/results/272/272520/final1-radish-2012.pdf)
- NASA KSC: two lettuce cultivars (Outredgeous, Flandria) grew less at 33 kPa than at 66 or 98 kPa. — [ASHS 2009 abstract](https://ashs.confex.com/ashs/2009/webprogram/Paper1648.html)
- Texas A&M (He, Davies et al.) found that 25 kPa total pressure does not hurt lettuce gas exchange or vegetative growth. Ethylene build-up above 1000 ppb in closed chambers did cut gas exchange and growth. pO2 treatments were 6, 12 and 21 kPa. — [ASHS 2011](https://ashs.confex.com/ashs/2011/webprogram/Paper7238.html); [Station report 2008](https://www.controlledenvironments.org/station-reports/texabae_2008); [Photosynthetica 2013 51:465](https://ps.ueb.cas.cz/artkey/phs-201303-0017_hypobaria-and-hypoxia-affects-phytochemical-production-gas-exchange-and-growth-of-lettuce.php)
- Literature reviewed by Zhou, Paul, Ferl et al. (Frontiers in Plant Science 2017):
  - About 50 kPa is the natural terrestrial limit for higher plants and the boundary between moderate and severe hypobaria.
  - Arabidopsis at 50 kPa and pO2 10.3 kPa: mild hypoxia, no obvious change in vegetative growth (Ramonell 2001).
  - Lettuce at 25–30 kPa with 2–6% O2 gave higher gas-exchange efficiency than 101 kPa at the same O2 (He 2007).
  - 30 kPa with pO2 6 kPa long term reduced growth (Tang 2015).
  - Arabidopsis at 10 kPa switched on hypoxia and drought/ABA genes even though fully hydrated (Paul et al. 2004).
  - Hypobaria speeds water movement through stomata.
  — [Zhou et al. 2017](https://frontiersin.org/journals/plant-science/articles/10.3389/fpls.2017.00528/full)
- 10 kPa compared with sea-level pressure changed the expression of more than 200 genes in Arabidopsis (Ferl group). — [PMC 316301](https://pmc.ncbi.nlm.nih.gov/articles/PMC316301)
- Ferl's group: at low pressure plants respond as if drying out, even with ample water and near-100% humidity. — [Mars Daily](https://www.marsdaily.com/reports/The_Greening_Of_Mars_Will_Be_An_Indoor_Affair.html)
- The 1999 workshop "Mars Greenhouses: Concepts and Challenges" (NASA/TM-2000-208577, eds. Wheeler & Martin-Brennan) covers low-pressure growth and CO2 as the pressurant. — [NTRS 20050182966](https://ntrs.nasa.gov/citations/20050182966)

**Gases, humidity, temperature (NASA BVAD design values)**
- Optimum pCO2 for plant growth is about 0.10–0.20 kPa; productivity drops below that. Plants lose productivity at very high pCO2 levels that are still acceptable for crew (up to about 1.0 kPa). Plants need RH of about 75% to avoid water stress. Wheat and potato do best below the crew comfort temperature range. At 0.04 kPa CO2, plants grow better with pO2 below 21 kPa, but with pCO2 at 0.1–0.2 kPa the benefit of reduced O2 is negligible. — [NASA Life Support Baseline Values and Assumptions Document (BVAD), NASA/TP-2015-218570 Rev1, Jan 2018](https://ntrs.nasa.gov/citations/20180001338)
- BVAD pCO2 for plants: lower 0.04 kPa, nominal 0.13 kPa, upper 3.4 kPa. Crew values for comparison: pCO2 nominal 0.507 kPa; cabin total pressure 48.0–102.7 kPa (nominal 101, 70.3 or 56.5); RH 25–75% (nominal 40%); temperature 291–300 K. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)

**Light, photoperiod, temperature per crop (BVAD Table 4-96)**

| Crop | PPF [mol/m²/d] | Photoperiod [h/d] | Growth period [days after planting] | Day/Night air °C |
|---|---|---|---|---|
| Lettuce | 17 | 16 | 28 | 23/23 |
| Spinach | 17 | 16 | 30 | 23/23 |
| Radish | 26 | 16 | 25 | 23/23 |
| Chard | 17 | 16 | 45 | 23/23 |
| Red beet | 17 | 16 | 40 | 23/23 |
| Green onion | 26 | – | 50 | 25/25 |
| Tomato | 27 | 12 | 85 | 24/24 |
| Pepper | 27 | – | 85 | – |
| Strawberry | 22 | 12 | 100 | 20/16 |
| Soybean | 28 | 12 | 97 | 26/22 |
| Dry bean | 24 | 18 | 85 | 28/24 |
| Peanut | 27 | 12 | 104 | 26/22 |
| Rice | 33 | 12 | 85 | 28/24 |
| Wheat | 115 | 20–24 | 75–90 | 20/20 |
| White potato | 28 | 12 | 132 | 20/16 |
| Sweet potato | 28 | 12 | 85 | 26/22 |

  — [BVAD 2018, Table 4-96](https://ntrs.nasa.gov/citations/20180001338)
- MEC crop models are valid for PPF 200–1000 µmol/m²/s (up to 2000 for rice and wheat) and CO2 330–1300 µmol/mol. Lettuce is limited to 200–500 µmol/m²/s, because a light integral of only about 17 mol/m²/d is recommended to prevent leaf tipburn. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Nominal model planting densities (plants/m²) and light/dark temperatures (BVAD Table 4-117):
  - lettuce 19.2, 16 h, 23/23 °C
  - wheat 720, 20 h, 23/23 °C
  - white potato 6.4, 12 h, 20/16 °C
  - soybean 35, 12 h, 26/22 °C
  - tomato 6.3, 12 h, 26/22 °C
  - rice 200, 12 h, 29/21 °C
  - sweet potato 16, 18 h, 28/22 °C
  - dry bean and peanut 7, 12 h, 26/22 °C

  — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- The Veggie (ISS) LED panel delivers 100–500 µmol/m²/s from red (630 nm), blue (455 nm) and green (530 nm) LEDs. — [NTRS Veggie 2011](https://ntrs.nasa.gov/api/citations/20110011606/downloads/20110011606.pdf)
- The Advanced Plant Habitat delivers 0–1000 µmol/m²/s PPF with red, blue, green, far-red and white channels. — [Frontiers in Plant Science 2020 (APH)](https://www.frontiersin.org/journals/plant-science/articles/10.3389/fpls.2020.00673/pdf)
- EDEN ISS (Antarctica) used custom high-power LEDs, mostly red with some blue. Air ran at 20–25 °C with slightly elevated CO2. Air was filtered and UV-sterilised, so no pesticides were needed. — [DLR EDEN ISS greenhouse page](https://www.dlr.de/en/research-and-transfer/projects-and-missions/eden-iss/the-eden-iss-greenhouse)
- Lunar Palace 1 lighting: plant cabin I ran 300–800 µmol/m²/s with continuous light (24/0); cabin II ran 250–800 µmol/m²/s on 12/12. Cabins held 22.3–22.7 °C and about 62% RH. — [Lunar Palace 365 preprint, bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf)

**Growth model usable in a simulator (NASA Modified Energy Cascade, MEC; Cavazzoni 2001, in BVAD)**
- Fraction of PPF absorbed by the canopy: A = A_MAX·(t/t_A)^n for t < t_A, and A_MAX afterwards, with A_MAX = 0.93. The exponent n is 1.0 for wheat; 1.5 for rice, soybean and sweet potato; 2.0 for dry bean, peanut and white potato; 2.5 for lettuce and tomato. t_A (canopy closure time) is a polynomial in PPF and CO2. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Daily carbon gain: DCG [mol C/m²/d] = 0.0036 × H × CUE24 × A × CQY × PPF. Here H is photoperiod (h), CUE24 is 24 h carbon use efficiency, CQY is canopy quantum yield and PPF is in µmol/m²/s. Crop growth rate CGR [g/m²/d] = 12.011 × DCG / BCF. O2 production = OPF × DCG. Edible biomass = XFRT × ∫CGR dt from t_E to t_M. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Constants:
  - CUE_MAX: lettuce 0.625, wheat 0.64, white potato 0.625, soybean 0.65, tomato 0.65, rice 0.64, peanut 0.65.
  - BCF: lettuce 0.40, wheat 0.44, potato 0.41, soybean 0.46, tomato 0.42.
  - OPF: lettuce 1.08, wheat 1.07, potato 1.02, soybean 1.16, tomato 1.09 mol O2/mol C.
  - Time constants as XFRT / t_E / t_Q / t_M in days after emergence:
    - lettuce 0.95 / 1 / n/a / 30
    - wheat 1.00 / 34 / 33 / 62
    - white potato 1.00 / 45 / 75 / 138 (harvested at 105)
    - soybean 0.95 / 46 / 48 / 86
    - tomato 0.70 / 41 / 56 / 80
    - rice 0.98 / 57 / 61 / 88
    - sweet potato 1.00 / 33 / n/a / 120
    - peanut 0.49 / 49 / 65 / 110
    - dry bean 0.97 / 40 / 42 / 63

  — [BVAD 2018, Tables 4-103 to 4-119](https://ntrs.nasa.gov/citations/20180001338)
- Low light keeps the canopy from closing. Lettuce at 200 µmol/m²/s and 330 ppm CO2 only reaches A_MAX 0.18 (0.49 at 1320 ppm); at 300 µmol/m²/s and 330 ppm it reaches 0.75. — [BVAD 2018, Table 4-129](https://ntrs.nasa.gov/citations/20180001338)

### Inferences
- A believable simulator greenhouse runs at roughly 10–50 kPa total pressure, with pO2 ≥ 5–7 kPa, pCO2 0.1–0.2 kPa (Martian air can supply it), 20–26 °C, RH 60–75% and PPFD 300–800 µmol/m²/s. Going below about 10 kPa should cause failure, mainly through water stress and hypoxia. Between 10 and 33 kPa, apply a growth penalty and raise transpiration, by up to about 6× at 10 kPa. Leave crew-shared habitats at 56–101 kPa.
- The MEC equations plus the constants above are a ready-made, NASA-sanctioned crop growth model. A simulator can integrate them per sol, with PPF, CO2 and photoperiod as live inputs from the greenhouse state.

### Gaps
- I did not obtain the full numeric coefficient tables for t_A and CQY_MAX (BVAD Tables 4-106 to 4-128). They are in the local BVAD PDF if needed.
- I found no long-duration seed-to-seed crop trial below 10 kPa. Most hypobaric data are short-term or vegetative only.
- I found no published design that combines natural Martian sunlight with a 10 kPa pure-CO2+O2 atmosphere in a full crop cycle.

---

## 3. Mars regolith and simulants: composition, perchlorates, nutrients, amendments, Wageningen

### Takeaway
Martian regolith is basaltic and has essentially no organic matter and no reactive nitrogen. It contains about 0.4–1 wt% perchlorate, which is toxic to plants: at 0.5–1 wt% germination stopped entirely in a simulant. The commonly used older simulant, JSC Mars-1A, is a Hawaiian volcanic tephra. With organic matter or manure added, it grows tomatoes, peas, rye, radish, cress and rocket about as well as poor Earth soil (Wageningen 2014–2019). The most mineralogically accurate simulant, MGS-1, is far harder to grow in: it is alkaline and its soluble phase inhibits germination. Recent work (2025–2026) fixes this with compost, insect frass and plant-growth-promoting or N-fixing bacteria. Perchlorate removal options are washing (water-expensive) or perchlorate-reducing microbes.

### Cited Findings
**Simulants**
- MGS-1 (Cannon et al. 2019, *Icarus* 317:470–478) is an open-standard simulant modelled on the Rocknest windblown soil at Gale crater. It is built from individual minerals based on Curiosity XRD mineralogy, including the X-ray amorphous component. — [Cannon et al. 2019 PDF](https://sciences.ucf.edu/class/wp-content/uploads/sites/23/2019/11/Cannon2019.pdf); [SSHADE](https://sshade.eu/data/publication/PUBLI_Cannon_2019)
- Current MGS-1 mineralogy in wt% (supersedes the 2019 paper): plagioclase 27.1, glass-rich basalt 22.9, pyroxene 20.3, olivine 13.7, Mg-sulfate 4.0, ferrihydrite 3.5, hydrated silica 3.0, magnetite 1.9, anhydrite 1.7, Fe-carbonate 1.4, hematite 0.5. No perchlorate is listed. — [UCF CLASS Exolith Lab, MGS-1](https://sciences.ucf.edu/class/?p=1216)
- There are variants: MGS-1S (sulfate-rich, 40% gypsum), MGS-1C (clay-rich, 40% smectite) and JEZ-1 (Jezero delta). UCF has stopped making them; Space Resource Technologies now sells them. — [UCF MGS-1S](https://sciences.ucf.edu/class/simulant_mgs1s/); [UCF MGS-1C](https://sciences.ucf.edu/class/simulant_mgs1c/); [UCF JEZ-1](https://sciences.ucf.edu/class/simulant_jez1/)
- JSC Mars-1(A), used by Wageningen in 2014, was mined on Hawaii (a volcanic cinder cone). It contains traces of nitrate and ammonium and a "significant amount of carbon", lacks phosphate, has high pH, and contains aluminium and chromium. — [Wamelink et al. 2014, PLOS ONE 9(8):e103138](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0103138); [Inside Science](https://www.aip.org/inside-science/astronauts-may-grow-better-salads-on-mars-than-on-the-moon)
- Florida Tech (Eichler, Hadland, Palmer, Wheeler; 2020/21) tested JSC-Mars-1A, Mars Mojave Simulant and MGS-1. Not all three supported plant growth, and MGS was toxic to plants despite attempts to make it viable. MGS had a higher pH, which "may have played a factor". All three were largely devoid of nitrogen. — [Florida Tech news, Aug 2020](https://news.fit.edu/academics-research/researching-regolith-for-plant-growth/)
- Murphy (Florida Tech MS thesis, May 2026) worked with cut-and-come-again lettuce in MGS-1. The water-soluble phase of MGS-1 lowers germination, and one constituent is concentrated enough to inhibit it. Germination was restored with plant-growth-promoting bacteria (PGPB), and a toxicity threshold was determined. The full text is embargoed until 9 Nov 2026. — [FIT repository etd/1671](https://repository.fit.edu/etd/1671)

**Perchlorate**
- Phoenix found 0.4–0.6% perchlorate ions in Martian soil. — [DLR elib 105915](https://elib.dlr.de/105915)
- About 0.6% of the icy regolith at 68°N was perchlorate. Perchlorate is hazardous to humans even at low concentration and corrodes equipment. Proposed fixes are microbial reduction and engineering the perchlorate-reduction pathway into spaceflight-tested *Bacillus subtilis*. — [Rothschild et al., LPSC 2025](https://epizodyspace.ru/bibl/inostr-yazyki/Lunar_and_Planetary_Science/2025/Rothschild_et_al_Detoxifying_Mars_56th_LPSC_no_2713_(2025).pdf)
- Oze et al. 2021 (*Soil Systems* 5(3):37, "Perchlorate and Agriculture on Mars") tested 0, 0.5 and 1 wt% perchlorate on grain amaranth and common bean, in a Gusev-crater simulant and in potting soil. In the simulant, any perchlorate stopped germination in all treatments. In potting soil, seeds sprouted at 1 wt% but grew poorly, with less leaf area and biomass. Perchlorate also increased metal and phosphorus release. — [MDPI Soil Systems 2021](https://www.mdpi.com/2571-8789/5/3/37); [DOAJ](https://doaj.org/article/234f1d4603434c22befcf972b74e288c)
- Beihang University microbially purified simulated Martian water of perchlorate. Perchlorate-sensitive wheat and lettuce irrigated with it reached 95% germination. — [BUAA](https://research.buaa.edu.cn/zh/publications/microbial-purification-of-perchlorate-in-a-simulated-martian-wate/)
- An anaerobic bioreactor student project used MGS-1 with acetate as electron donor plus a perchlorate-reducing inoculum. — [ASU FURI 2026](https://forge.engineering.asu.edu/wp-content/uploads/2026/03/Lopez-Torres_FURI.pdf)
- A review of microbial remediation of Martian regolith and phytoremediation candidates (2020). — [DOI 10.1089/space.2020.0055](https://journals.sagepub.com/doi/10.1089/space.2020.0055)
- Andy Weir says perchlorates "can literally just rinse" out. The catch is water: wash outside, then boil off and recapture the water. — [Modern Farmer 2015](https://modernfarmer.com/2015/10/can-you-grow-plants-on-mars/)

**Wageningen "Food for Mars and Moon" (Wieger Wamelink)**
- 2014 PLOS ONE (experiment started 8 Apr 2013, ran 50 days):
  - Design: 14 species (4 crops, 4 N-fixers, 6 wild plants), 840 pots (3 soils × 14 species × 20 replicates), 5 seeds per pot. Pots held 50 g Mars simulant (JSC Mars-1A), 100 g Moon simulant (JSC-1A) or 100 g Rhine river sand control.
  - Conditions: glasshouse at 21.1 ± 3.0 °C, RH 65 ± 15.5%, 16 h day with lamps on below 150 W/m² of sunlight. No nutrients added.
  - Results: Mars simulant gave the highest germination, survival and leaf formation, and significantly higher biomass than both other soils for 11 of 14 species. Tomato, wheat and cress flowered on Mars simulant; cress and field mustard set seed (field mustard only on Mars). On Moon simulant only one bud formed.
  - Mars simulant held water best.

  — [Wamelink et al. 2014](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0103138)
- 2019 Open Agriculture 4(1) (sown April 2015):
  - Crops: 10 species (garden cress, rocket, tomato, radish, rye, quinoa, spinach, chives, pea, leek) in Mars simulant (JSC Mars-1A), Moon simulant (JSC-1A) and Earth potting soil, all with organic matter added to stand in for crop residue.
  - Results: 9 of 10 grew well; spinach failed. Total biomass on Mars simulant was comparable to Earth soil and significantly higher than on Moon simulant. Seeds of radish, rye and cress germinated, and enough seed was harvested to start a second generation. Germination of radish seed from Moon simulant was significantly lower.

  — [Sci.News 2019](https://www.sci.news/space/crops-simulated-martian-lunar-dirt-07697.html); [DOAJ summary](https://doaj.org/article/b93b1deadbd644ad937da1d9302ac076); [DOI 10.1515/opag-2019-0051](https://doi.org/10.1515/opag-2019-0051)
- Heavy metals (2016): the simulants contain heavy metals such as lead, mercury and chromium. Tomato, radish, pea and rye were tested for heavy metals and alkaloids before a "Mars dinner", and levels were far below the maximum allowable. Wamelink concluded the crops were safe to eat. — [WUR news, Mars and Moon dinner](https://www.wur.nl/en/newsarticle/Dinner-of-Mars-and-moon-vegetables-well-received.htm); [HortiDaily](https://www.hortidaily.com/article/6028591/nl-dinner-of-mars-and-moon-vegetables-well-received); [ScienceAlert](https://www.sciencealert.com/earthworms-can-reproduce-in-fertilised-mars-soil-simulant)
- Earthworms (Nov 2017): rucola (rocket) was grown in Mars simulant with manure, and earthworms were added. Two young worms were found in the Mars simulant at the end, showing the worms reproduced. Manure boosted growth, especially in Mars simulant, which with manure outperformed Earth silver sand. Planned next steps were pig manure and perchlorate tests. — [ScienceAlert, 28 Nov 2017](https://www.sciencealert.com/earthworms-can-reproduce-in-fertilised-mars-soil-simulant); [WUR news](https://www.wur.nl/en/newsarticle/Earthworms-can-reproduce-in-Mars-soil-simulant.htm)
- The project lists green beans, peas, radish, tomato, potato, rucola, carrot and garden cress as apparently possible. — [Digital Trends](https://uat4.www.digitaltrends.com/?p=1280981)

**Other regolith-amendment results**
- Villanova (Guinan, 2018) used a commercial basalt simulant described as about 93% similar to Martian regolith, with no perchlorate, plus inorganic fertiliser. They loosened it with shredded cardboard or vermiculite (about 1/3 filler for potatoes) and grew under 44% Earth light. Kale, sweet potato, some lettuces and hops grew very easily. Russet potato grew only after soil treatment. Peas and spinach did poorly because the greenhouse was too hot. — [Eos 2018](https://eos.org/articles/tests-indicate-which-edible-plants-could-thrive-on-mars)
- Peas (Nov 2025) grew in MMS-1 simulant amended with black soldier fly frass over two 8-week trials. Germination and height matched soil. The best frass inclusion was about 5–32% depending on the metric. Biomass was below commercial potting mix. — [Astrobiology.com, 28 Nov 2025](https://astrobiology.com/2025/11/28/peas-on-mars-a-study-of-garden-pea-lathyrus-oleraceus-pisum-sativum-growth-in-martian-regolith-simulant-treated-with-black-soldier-fly-frass/)
- Frontiers in Plant Science (2025) compared six substrates. Pure simulant reduced photosynthesis and biomass. Compost-amended simulant raised seed production by 61.9% over pure regolith, matching volcanic soil and sand-compost. — [Frontiers 2025, fpls.2025.1676285](https://public-pages-files-2025.frontiersin.org/journals/plant-science/articles/10.3389/fpls.2025.1676285/text)
- Treating seeds and irrigation water with plasma-activated water plus feldspar-derived minerals raised microgreen germination by about 567% and biomass by 412% in simulant (ANU, 2025). — [ANU research portal](https://researchportalplus.anu.edu.au/en/publications/enhanced-plant-growth-on-simulated-martian-regolith-via-water-che/)
- Clover inoculated with *Sinorhizobium meliloti* grew 75% more root and shoot in regolith simulant than uninoculated clover. — [The Debrief](https://thedebrief.org/new-astroagriculture-research-may-have-just-solved-how-we-grow-food-on-mars/)
- Sweet potato has been grown in MGS-1 as a space model plant. — [Plants 13(1):55, MDPI](https://0-www-mdpi-com.brum.beds.ac.uk/2223-7747/13/1/55)
- NASA BVAD: feeding one person (2500 kcal/d) hydroponically would take about 90–100 kg of fertiliser salts per person per year. More than 50% of these nutrients could be recycled from inedible biomass by composting or stirred-tank reactors, and from processed urine. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)

### Inferences
- For a simulator, a "raw regolith" bed should have zero N, low available P, high pH (about 8–9 for MGS-1-like material; inferred from "higher pH", no number found) and perchlorate at 0.5–1 wt%. With perchlorate present, germination should be zero. After washing or bioremediation plus compost, manure or frass (about 5–30% by volume), plus N-fixer inoculation for legumes, growth can approach poor Earth soil. Earthworms are a plausible amendment agent.
- Treat JSC Mars-1A results as optimistic: it is a weathered Hawaiian tephra with organic carbon, and it is easier than real regolith or MGS-1.

### Gaps
- I could not open the full 2019 Open Agriculture paper (publisher blocked). I have no per-crop biomass numbers, organic-matter fraction or exact conditions for it.
- I found no published Wageningen perchlorate results, though they were planned in 2017.
- I found no quantitative washing protocol (water volume per kg of regolith to get below a safe perchlorate level).
- MGS-1 bulk oxide chemistry, particle size and pH values were not on the UCF page.

---

## 4. Crops for space and Mars: cycle lengths, yields, harvest index, calories, area per person

### Takeaway
NASA's design numbers (BVAD 2018, built on the KSC Biomass Production Chamber) give edible dry productivity of about 20 g/m²/d for white potato and wheat, about 25 for sweet potato, 6.6 for lettuce and 4.5 for soybean. Harvest indices range from 90% for lettuce down to 40% for wheat and soy. Cycles run 25–30 days for radish and lettuce, 62–90 for wheat and about 105–132 for potato. Feeding one person fully takes roughly 40–50 m² of high-light crop area. Supplying one person's O2 takes about 20–25 m². Early missions will have under 5 m², which only supplies fresh salad.

### Cited Findings
- BVAD Table 4-97, nominal productivity at maturity. Columns: height | harvest index | edible g dry/m²/d | edible g fresh/m²/d | fresh water %. Source: [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
  - Lettuce 0.25 m | 90% | 6.57 | 131.35 | 95%
  - Spinach 0.25 | 90% | 6.57 | 72.97 | 91%
  - Radish 0.20 | 50% | 5.50 | 91.67 | 94%
  - Cabbage 0.35 | 90% | 6.06 | 75.78
  - Carrot 0.25 | 60% | 8.98 | 74.83
  - Tomato 0.40 | 45% | 10.43 | 173.76 | 94%
  - Pepper 0.40 | 45% | 10.43 | 148.94
  - Strawberry 0.25 | 35% | 7.79 | 77.88
  - Wheat 0.50 | 40% | 20.00 | 22.73 | 12%
  - Rice 0.80 | 30% | 9.07 | 10.30
  - Soybean 0.55 | 40% | 4.54 | 5.04
  - Peanut 0.65 | 25% | 5.63 | 5.96
  - Dry bean 0.50 | 40% | 10.00 | 11.11
  - Pea 0.50 | 40% | 10.73 | 12.20
  - White potato 0.65 | 70% | 21.06 | 105.30 | 80%
  - Sweet potato 0.65 | 60% | 24.7 | 51.72 | 71%
- BVAD Table 4-98, total dry biomass (nominal / highest recorded) in g/m²/d, then O2 production / CO2 uptake (g/m²/d) and transpiration (kg/m²/d). Source: [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
  - Lettuce 7.30 / 7.9; O2 7.78, CO2 10.70; 2.10 kg/m²/d
  - Wheat 50 / 150; O2 56.0, CO2 77.0; 11.79 kg/m²/d
  - White potato 30.08 / 50; O2 32.23, CO2 45.23; 4.00 kg/m²/d
  - Soybean 11.34 / 20; O2 13.91; 4.70 kg/m²/d
  - Tomato 23.17 / 37.8; O2 26.36; 2.77 kg/m²/d
  - Rice 30.23 / 39; O2 36.55; 3.43 kg/m²/d
  - Sweet potato 37.5 / 51.3; O2 41.12; 2.88 kg/m²/d
  - Radish 11.0; 1.77 kg/m²/d
- Water and nutrients per g of dry biomass (Wheeler et al. 1999): water 0.32 L (soy), 0.13 (wheat), 0.15 (potato), 0.34 (lettuce). Stock solution 0.021–0.034 L. Acid (0.4 M HNO3) 0.043–0.074 g. — [BVAD Table 4-100](https://ntrs.nasa.gov/citations/20180001338)
- The BPC data are conservative because they used fixed spacing. Transplanting or variable spacing saves up to 15 days per cycle (lettuce 28 → about 14 days in the production bed) and raises productivity by about 10–15%. Seedling nurseries need about 1–10% of production area. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Growing area per crew member for BVAD diets (Table 4-99): salad-crops-only diet 1.35 m²/CM; salad plus carbohydrate crops 19.5 m²/CM; diet using all ELS crops 65.29 m²/CM. In the all-crop diet, soybean alone is 46.4 m², wheat 4.24 and peanut 4.83. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- If more than about 25% of food (dry mass) is grown locally, transpiration can regenerate all the water. At about 50% or more, crops can regenerate all the air (Drysdale et al. 1997). The design crew energy requirement is 12.99 MJ/CM-d (82 kg reference body mass). — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Larger bioregenerative systems for planetary surfaces were estimated at about 40–50 m² (or more) of crop area per person. Early missions may have under 5 m². — [Johnson et al. 2021, Frontiers in Astronomy and Space Sciences 8:734343](https://www.frontiersin.org/journals/astronomy-and-space-sciences/articles/10.3389/fspas.2021.734343/text)
- Wheeler: 20–25 m² of crops could provide all of one person's O2, and 40–50 m² all of their dietary calories. Energy-efficient lighting is key to high yields. — [Wheeler presentation (SIMOC)](https://simoc.space/wp-content/uploads/2018/09/When-Can-Plants-Begin-to-Contribute-to-Atmospheric-Management-WHEELER.pdf)
- The KSC Biomass Production Chamber ran over 1,200 days with wheat, soybean, lettuce and potato. Edible yields and harvest indices came in slightly under expectation, while total biomass was close to predictions. — [NTRS 20040089951](https://ntrs.nasa.gov/citations/20040089951)
- It had 20 m² of growing area on four stacked 5 m² shelves, ran 1987–2000, and was lit by 96 × 400 W high-pressure sodium lamps. — [NASA NTRS Wheeler documents](https://ntrs.nasa.gov/api/citations/20190002770/downloads/20190002770.pdf)
- Sweet potato in an 11.2 m² chamber over 17 weeks reached a harvest index of about 38.6% fresh-mass basis. — [NTRS 20100039474](https://ntrs.nasa.gov/api/citations/20100039474/downloads/20100039474.pdf)
- NASA dwarf-cultivar ground screening grew dwarf tomato to fruiting age in up to 106 days and pepper in 109 days; 'Red Robin' tied for first of six tomato cultivars. — [NTRS 20190027725](https://ntrs.nasa.gov/citations/20190027725)
- Lunar Palace 365 crop productivity was generally below BVAD values, which the authors attribute to cultivar differences. — [bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf)

### Inferences
- Calories per m² (my calculation; energy densities are standard food values, not from the sources above): potato edible dry matter at about 21 g/m²/d × about 3.6–3.8 kcal/g ≈ 75–80 kcal/m²/d. Wheat grain at about 20 g/m²/d × about 3.4–3.9 kcal/g ≈ 70–78 kcal/m²/d. Feeding 2500–3000 kcal/d from these staples therefore needs about 33–40 m² of high-light area, consistent with the 40–50 m² NASA figure once diet diversity and nursery space are included. Lettuce gives only about 20 kcal/m²/d and is grown for vitamins and morale.
- Simulator yield calibration: fresh harvest per m² per cycle = fresh productivity × cycle days. Examples: lettuce 131 g/m²/d × 28 d ≈ 3.7 kg/m²; potato 105 g/m²/d × 132 d ≈ 13.9 kg/m² of tubers; wheat 22.7 g/m²/d × 80 d ≈ 1.8 kg/m² of grain; tomato 174 g/m²/d × 85 d ≈ 14.8 kg/m². These are ideal-chamber values at the BVAD light levels. Natural Mars light or poor regolith should give less.
- Transpiration is huge: wheat at about 12 kg/m²/d, potato about 4, lettuce about 2 (BVAD), and Lunar Palace measured 6.13 L/m²/d on average. Condensate recovery is therefore the main water loop in any Mars greenhouse model.

### Gaps
- I found no measured per-m² yields in Martian regolith simulant from Wageningen (only relative biomass).
- I found no microgreen cycle and yield data in BVAD. Microgreens (radish microgreens and so on) appear in Florida Tech plans but without numbers in the sources I opened.

---

## 5. Major real systems and analogues

### Takeaway
All flight and analogue systems except Biosphere 2 are soil-free: hydroponic or aeroponic, under LEDs (HPS lamps in the older BPC). ISS Veggie and APH each have about 0.17 m² and have shown lettuce, mizuna, cabbage, radish, zinnia, chile pepper and dwarf tomato in microgravity. EDEN ISS (12.5 m², Antarctica) produced 268 kg of fresh food in about 9.5 months in 2018, using about 0.8 kW of power per m². Lunar Palace 1 (120 m² of planting area, 4 crew) achieved 100% O2 and water regeneration and 73% (dry) food regeneration over 370 days. Biosphere 2 (2000 m² of soil farming) gave about 80% of food but cost 10–20% crew body weight loss. Wageningen and CIP are the main regolith or soil-analogue experiments.

### Cited Findings
**NASA Veggie (ISS, since 2014)**
- Growth area about 0.17 m² (a 2011 abstract gives 0.13 m²). Baseplate 29.2 × 36.8 cm, maximum height 47 cm. LED 100–500 µmol/m²/s (red 630, blue 455, green 530 nm). Plants grow in "pillows" of about 100 mL of media, six per unit, watered passively by wicking. — [NTRS 2011](https://ntrs.nasa.gov/api/citations/20110011606/downloads/20110011606.pdf); [ASHS 2011](https://ashs.confex.com/ashs/2011/webprogram/Paper7418.html)
- VEG-01 grew 'Outredgeous' red romaine lettuce (first test crop, 33-day grow-outs; grown over 33–56 day periods 2014–2016). The crew first ate it in August 2015. Returned lettuce was at least as nutritious as Earth controls. Zinnias tested a longer flowering crop. Watering (too little or too much) was a recurring problem. — [NTRS 20160004060](https://ntrs.nasa.gov/citations/20160004060); [Awkward Botany](https://awkwardbotany.com/tag/veggie/)
- Crops grown in Veggie include lettuce, Swiss chard, radish, Chinese cabbage and peas. — [ScienceABC summary](https://www.scienceabc.com/nature/universe/how-they-grow-plants-in-space-iss-veggie-zero-gravity). NASA planning documents designated VEG-04 for mizuna and VEG-05 for 'Red Robin' dwarf tomato, with light-quality treatments. — [NTRS 20180000694](https://ntrs.nasa.gov/citations/20180000694)
- VEG-05 'Red Robin' dwarf tomato ran its planned 100 days, with the final harvest on 29 March 2023. A station humidity drop early on stressed the plants, and NASA advised against eating the fruit over fungal concerns. — [NASA](https://www.nasa.gov/missions/station/nasa-teams-persevere-through-plant-challenges-in-space); [Live Science](https://livescience.com/space/space-exploration/nasa-astronauts-finally-find-1-inch-tomato-that-was-lost-in-space-for-8-months)

**NASA Advanced Plant Habitat (ISS)**
- Growth area 1708 cm² (about 0.17 m²) in a 0.2 m² instrumented root module. Shoot height 45 cm, root zone 5 cm, growth volume 112,500 cm³. PPF 0–1000 µmol/m²/s from red, blue, green, far-red and white LEDs. More than 180 sensors, controlled from KSC. — [Frontiers in Plant Science 2020](https://www.frontiersin.org/journals/plant-science/articles/10.3389/fpls.2020.00673/pdf); [NASA PH-04](https://www.nasa.gov/?p=70033)
- PH-04 grew a dwarfed Hatch chile (*Capsicum annuum*) from four plants for 137 days; harvests ran from late October to 26 November 2021. Peppers are hard because they need hand pollination and about 4 months to grow. — [NASA](https://www.nasa.gov/?p=70033); [Digital Trends](https://www.digitaltrends.com/space/nasa-chile-pepper-project-iss/)

**DLR EDEN ISS (Neumayer III, Antarctica; operations from 2018)**
- Mobile Test Facility: two 20 ft containers (one service section, one "Future Exploration Greenhouse" with about 12.5 m² of cultivation area) about 300 m from the station. It used aeroponics (roots misted every few minutes) and NFT, LEDs, CO2 enrichment, a nearly closed water loop, and operation from a remote control room with image-based diagnosis. — [eden-iss.net](https://eden-iss.net/?p=5381); [DLR greenhouse page](https://www.dlr.de/en/research-and-transfer/projects-and-missions/eden-iss/the-eden-iss-greenhouse)
- 2018 season (Paul Zabel): 268 kg of food from 12.5 m² in about 9.5 months, including 67 kg cucumber, 117 kg lettuce/leafy greens and 50 kg tomato. The paper version lists 56 kg lettuces + 49 kg leafy greens, plus herbs, radish and kohlrabi. Overall productivity was 27.4 kg/m²/yr. It fed an overwintering crew of 10. — [DLR press release, 13 Sep 2018](https://www.dlr.de/en/latest/news/2018/3/20180913_greenhouse-braves-the-antarctic-dlr-researcher-paul-zabel-experiences-record-harvest-and-instructive-challenges_29756); [DLR elib 135980](https://elib.dlr.de/135980)
- Mean power use was 0.8 kW per m² of cultivated area, against the 2.1 kW/m² previously assumed for space greenhouses. — [IO+](https://ioplus.nl/archive/en/eden-iss-substantial-vegetable-harvest-in-antarctica/)
- Crops: tomato, pepper, cucumber, several lettuces, radish, spinach, basil, chives, parsley, mint, coriander, strawberry. — [DLR](https://www.dlr.de/en/research-and-transfer/projects-and-missions/eden-iss/the-eden-iss-greenhouse)

**China Lunar Palace 1 / "Lunar Palace 365" (Beihang University, 10 May 2017 – May 2018)**
- Facility: 160 m², 500 m³; two plant cabins plus one living cabin; leak rate 0.043%/day. Crew of 8 volunteers in rotating groups of 4, over 370 days total (one group stayed 200 days). — [bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf); [BUAA/Xinhua](https://news.buaa.edu.cn/info/1006/43731.htm)
- Plants: 120 m² of planting area on three tiers, 35 species (5 staple crops, 29 vegetables, 1 fruit). Cabin I used vermiculite or a soil-like substrate made from solid waste, under continuous 300–800 µmol/m²/s. Cabin II was hydroponic on 12/12 at 250–800 µmol/m²/s. Both used Hoagland solution. Wheat had the largest area and was the main O2 producer. — [bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf)
- Outcomes:
  - 100% O2 and water regeneration; O2 held at 20.1–21.5% (mean 20.71%).
  - CO2 ranged 246–4131 ppm (mean 1241 ppm), against a plant design target of 1300 ppm.
  - Total transpiration 724 L/d, or 6.13 L/m²/d.
  - Urine water recovered with 99.7% N recovery; solid-waste recovery 67%.
  - Food regeneration 73% dry weight (83% fresh weight, 100% of plant food). Yellow mealworms fed on inedible leaves supplied animal protein.
  - Overall closure 98.2%.
  - Food regeneration (dry) in earlier systems: the 3-person 105-day Lunar Palace run (2014) 55% (overall closure 97%); a 4-person 180-day Chinese CELSS (2016, 260 m²) 55%; the 3-person 90-day BIOS-3 (Russia) 48%; Japan's 2-person 28-day CEEF (150 m²) 92–95%.
  - In an earlier 2-person, 30-day "lettuce-human" CELSS test, 13.5 m² of planting met one person's O2 demand.

  — [bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf)

**ESA MELiSSA (Micro-Ecological Life Support System Alternative)**
- Five-compartment loop. The early compartments handle microbial waste degradation, C3 is nitrification, C4a (cyanobacteria) and C4b (higher plants) revitalise the air and produce food, and C5 is the crew, with rats as the mock crew for cost and safety reasons. — [MELiSSA Foundation pilot plant](https://www.melissafoundation.org/page/melissa-pilot-plant)
- The pilot plant at UAB Barcelona has been an ESA external lab since 2009. A 2021 paper demonstrated integrated C3 + C4a + C5 with rats, with O2 producer and consumer balanced in transient and steady states. Integration of C4b (higher plant chamber) was ongoing. Candidate HPC crops are lettuce, beet and wheat. — [Frontiers 2021](https://www.frontiersin.org/journals/astronomy-and-space-sciences/articles/10.3389/fspas.2021.750616/pdf); [UAB pilot plant](https://webs.uab.cat/melissapilotplant/en/?p=107)

**Biosphere 2 (Arizona, mission 1, 1991–1993)**
- About 2000 m² of soil-based cropping fed the sealed crew for 2 years and supplied about 80% of nutrition (Wikipedia cites 83%). The two-year average diet was about 2200 kcal, 73 g protein and 32 g fat per person per day, mostly vegetarian. Agriculture and food processing took about 45% of crew time. Crew lost 10–20% of body weight, mostly in the first six months. — [Silverstone & Nelson 1996 via ecotechnics.edu](https://ecotechnics.edu/?p=189); [Wikipedia](https://www.wikipedia.com/wiki/Biosphere2)
- Mark Nelson: intake started around 1800 kcal/d and rose to about 2200 as farming improved; farming took about 25% of waking time. — [Dartmouth Alumni Magazine](https://dartmouthalumnimagazine.com/articles/biosphere-2-what-really-happened)

**Mars Desert Research Station GreenHab (Mars Society, Hanksville, Utah)**
- The GreenHab is a climate-controlled, solar-powered greenhouse with conventional and aquaponic beds and supplementary winter lighting. Crews include a "GreenHab Officer". A recent crew tested cyanobacteria (blue-green algae) as a growth aid in NASA-type Mars simulant, with one seed sprouting. — [KUTV](https://kutv.com/news/eye-on-utah/what-are-they-doing-at-mars-site-in-utah-southeast-desert-hanksville-wayne-county-mars-society-elon-musk-foundation); [Analog training site](https://analog-training.newmars.com/); [Space.com](https://www.space.com/growing-plants-on-mars-analog-desert.html)

**University of Arizona CEAC Lunar/Mars Greenhouse prototype**
- A 5.5 m long, 1.8 m diameter inflatable membrane cylinder using hydroponic "Cable Culture" (about 21 m³ plus 2 m³ HVAC). Crops: lettuce, tomato, sweet potato, strawberry or cowpea. — [UA LGH Phase I](https://cales.arizona.edu/lunargreenhouse/PhaseI.htm); [Giacomelli 2012 slides](https://cales.arizona.edu/lunargreenhouse/Documents/2012-07-20_01_Giacomelli.pdf)
- Measured: biomass gain 0.06 ± 0.01 kg/m²/d (wet), condensate about 21.4 L/d, electricity about 100.3 kWh/d. Projected: 4 modules would give a 4-person crew 100% water and air recycling and about 50% of calories (about 1000 kcal per person). — [UA LGH Phase II](https://cales.arizona.edu/lunargreenhouse/PhaseII.htm); [Giacomelli 2012](https://cales.arizona.edu/lunargreenhouse/Documents/2012-07-20_01_Giacomelli.pdf)

**International Potato Center (CIP), "Potatoes on Mars", Lima, Peru (2016–2017)**
- Phase 1 (2016): potatoes grew in very dry, salty Pampas de La Joya desert soil from southern Peru once fertilised Earth soil was added. A top performer was a highly salt-tolerant variety later released for saline coastal Bangladesh. Chris McKay (NASA Ames) called these "the most Mars-like soils found on Earth". — [CIP blog, 8 Mar 2017](https://cipotato.org/blog/indicators-show-potatoes-can-grow-mars/)
- Phase 2: a tuber was planted on 14 February 2016 in a sealed CubeSat container built by UTEC to NASA Ames designs. It received nutrient water automatically, a day/night temperature cycle and Mars-like pressure, O2 and CO2, with live cameras. Preliminary results were "positive" and the stress-tolerant potatoes "were able to produce tubers". — [CIP blog](https://cipotato.org/blog/indicators-show-potatoes-can-grow-mars/)

### Inferences
- For a simulator, EDEN ISS is the best real calibration point for a Mars-surface "container greenhouse": 12.5 m², about 27 kg fresh food per m² per year, about 0.8 kW/m², aeroponic, crew of 10 supplemented (not fed). Lunar Palace is the calibration point for a "full closure" farm: 120 m² for 4 people, about 30 m² per person, about 73% of food.
- The ISS units are too small (0.17 m²) for food production. They matter for the habitat's microgravity-free analogue only in watering and pollination lessons.

### Gaps
- CIP never published (in sources found) the actual CubeSat pressure, temperature or CO2 values, or tuber yield. Press reports of the conditions are not technical.
- I found no MELiSSA higher-plant yield or closure numbers.
- I found no MDRS GreenHab yield data.
- I found no EDEN ISS PPFD, CO2 or RH setpoints in the sources accessed.

---

## 6. Hydroponics vs aeroponics vs regolith-based growing on Mars

### Takeaway
Every agency system flown or tested at scale is soil-free. NASA BPC, Veggie, APH and Lunar Palace cabin II are hydroponic or substrate-based. EDEN ISS is aeroponic, and MELiSSA uses hydroponic chambers. Hydroponics has the highest technology readiness and the best-quantified yields. Regolith is attractive for scaling (only seeds come from Earth) and buffers the root zone. It needs perchlorate removal plus N and organic amendment, has almost no flight-relevant data, and has never been compared head-to-head with hydroponics for the same space crop.

### Cited Findings
- The technology readiness of hydroponics for space agriculture surpasses that of regolith-based agriculture, and few studies show regolith as a viable substrate "as is". The dissertation instead explores regolith as one component of a hydroponic substrate. — [Florida Tech dissertation 2026, etd/1663](https://repository.fit.edu/etd/1663)
- No direct comparison between hydroponic and regolith systems exists for any crop targeted for space (Palmer, 2023). A Planetary Society STEP grant ($50k, 2023) funds side-by-side trials of radish microgreens, lettuce and tomato. The hypothesis is that fast crops suit hydroponics and slow crops such as tomato favour regolith. Hydroponics carries a risk of fungal and bacterial contamination that worsens over time. Regolith is sterile but lacks nitrogen and is salt-rich. — [Planetary Society](https://www.planetary.org/sci-tech/growing-veggies-moon-mars); [Florida Tech news](https://news.fit.edu/science/palmer-wins-planetary-society-grant-to-study-deep-space-agriculture/)
- Argument for regolith: scaling hydroponics needs far more manufacturing infrastructure than scaling amended-regolith farming, if remediation is simple (opinion essay). — [Centauri Dreams, Jan 2023](https://www.centauri-dreams.org/2023/01/26/food-production-on-mars-dirt-farming-as-the-most-scalable-solution-for-settlement/)
- Wageningen's argument: growing in regolith would mean only seeds have to be shipped from Earth. — [Modern Farmer](https://modernfarmer.com/?p=68813)
- EDEN ISS aeroponics: roots hang in closed containers and are sprayed every few minutes, with no soil. Air filtration plus UV avoided pesticide use. — [DLR](https://www.dlr.de/en/research-and-transfer/projects-and-missions/eden-iss/the-eden-iss-greenhouse)
- NASA BPC hydroponic nutrient management: replenishment solution is added according to electrical conductivity, pH is controlled with nitric acid, and the solution is periodically replaced to stop Na and B building up. — [BVAD 2018](https://ntrs.nasa.gov/citations/20180001338)
- Lunar Palace ran both modes side by side: substrate (vermiculite or a soil-like substrate from waste) in cabin I and hydroponics in cabin II. — [bioRxiv 2021](https://www.biorxiv.org/content/10.1101/2021.01.12.426282.full.pdf)
- Veggie uses passive wicking pillows with substrate. Watering failures (too much or too little) were the main problem in early tests. — [NTRS 20160004060](https://ntrs.nasa.gov/citations/20160004060)
- A 1991 NASA CELSS paper compared liquid hydroponics with lunar regolith as a substrate (abstract only). — [NTRS 19910030111](https://ntrs.nasa.gov/citations/19910030111)

### Inferences
- A believable Mars-base progression for a simulator: (1) early hydroponic or aeroponic racks under LEDs inside the pressurised habitat (EDEN-ISS-like, 0.8 kW/m²); (2) later, washed and bioremediated regolith beds amended with compost from crop residue and treated waste, with N-fixing legumes and worms, for bulk staples such as potato and wheat; (3) a hybrid in which regolith is an inert hydroponic substrate component. Robots would plausibly handle regolith excavation, washing, sieving and mixing, plus rack handling.

### Gaps
- I found no published head-to-head hydroponic-versus-regolith yield data as of October 2026. The Palmer results are not yet in the sources found.
- I found no explicit statement of a NASA or ESA policy preference. The preference is inferred from what they have built.

---

## 7. The Martian (Andy Weir novel; 2015 film): Watney's potatoes and what is accurate

### Takeaway
Watney turns the Hab floor (and bunks and pop-tents) into about 126 m² of potato beds 10 cm deep. He uses regolith and the crew's freeze-dried faeces as fertiliser and biological starter. He makes extra water by catalytically decomposing hydrazine (N2H4 → N2 + 2H2 over iridium) and burning the hydrogen. Experts accept the outline: regolith needs nutrients and biology, waste is a plausible fertiliser, and the hydrazine chemistry is real. The book's big miss is perchlorate, which Weir didn't know about. Other weak points: fuzzy volume-based stoichiometry, an under-shielded "greenhouse", and potato timelines that are optimistic.

### Cited Findings
- Farm size: covering the Hab floor, unused bunks and pop-tents gives 126 m² of farmable land and an estimated about 850 kcal/day. One summary instead uses 62 m² and about 288 kcal/day; the two figures conflict and should be checked against the novel. — [LitCharts ch. 3](https://www.litcharts.com/lit/the-martian/chapter-3); [LitCharts ch. 2](https://www.litcharts.com/lit/the-martian/chapter-2)
- Soil: cover the 92 m² Hab floor with 10 cm of soil, about 9.2 m³ hauled through the airlock. It needs about 40 L of water per m³, about 618 L total, against about 300 L available. — [Space.com infographic](https://space.com/30776-math-in-the-martian-movie-explained-infographic.html) (from search snippet; page did not render)
- In the story, potatoes are harvested after 48 sols in a plastic-sheet enclosure built over 126 m² of soil. — [Warwick Knowledge Centre](https://warwick.ac.uk/newsandevents/knowledgecentre/science/physics-astrophysics/growing_potatoes_on_mars) (from search snippet)
- Water chemistry: hydrazine decomposes catalytically over iridium to N2 and H2, and the H2 is combined with O2 to make water. Leftover H2 built up in the Hab and was burned off in controlled bursts, later causing an explosion. — [LSU Chemistry in "The Martian"](https://faculty.lsu.edu/maverick/martian.php); [LitCharts ch. 4](https://www.litcharts.com/lit/the-martian/chapter-4)
- A critic notes Watney reasons in volumes rather than masses, which is wrong for stoichiometry, and that burning stoichiometric H2/O2 is very dangerous. — [blog.des.no](https://blog.des.no/?p=969)
- Weir: "The main thing that's not in Martian soil is a bunch of nutrients and biological materials that plants rely on to grow". Using your own waste is acceptable "in a desperate survival situation"; freeze-dried waste pathogens "would have been dead". He did not know about perchlorates when writing, and says they could be rinsed out. — [Modern Farmer, 2 Oct 2015](https://modernfarmer.com/2015/10/can-you-grow-plants-on-mars/)
- Perchlorates would either hinder growth or make the plants toxic. Washing requires water to be boiled off and recaptured. Ryan Miller (U-Michigan) noted that the Hab enclosure would need to block radiation. — [Engineering.com](https://www.engineering.com/?p=13185); [Modern Farmer](https://modernfarmer.com/2015/10/can-you-grow-plants-on-mars/)
- Later work: some microbes tolerate or use perchlorate, for example in the Atacama. A 2023 LPSC abstract proposed gene-edited rice as an alternative to potatoes. — [U-Arkansas Research](https://arkansasresearch.uark.edu/new-findings-indicate-gene-edited-rice-might-survive-in-martian-soil)
- Real-world counterpart: CIP's 2017 potato work was explicitly motivated by Mars and used salt-tolerant cultivars in a Mars-analogue desert soil. — [CIP](https://cipotato.org/blog/indicators-show-potatoes-can-grow-mars/)

### Inferences
- Comparing the novel with the data (my reasoning): the BVAD potato value of 105 g fresh/m²/d with a 105–132 day cycle implies about 11–14 kg of tubers per m² per cycle at about 28 mol/m²/d of light. If 126 m² averaged about 75–80 kcal/m²/d (see section 4), that would be about 9,500–10,000 kcal/d. The novel's about 850 kcal/d is therefore very conservative for an ideal chamber, but plausible for dim Hab light, a 10 cm root zone and untreated regolith. A first harvest at 48 sols is early compared with BVAD's 105-day potato harvest, though Watney was propagating seed potatoes, not reaching full yield.
- The real accuracy problems for a simulator scenario are perchlorate (germination stops at 0.5–1 wt% in simulant), nitrogen (none in regolith), and pathogen risk from raw human waste.

### Gaps
- I could not render the full Space.com infographic or Warwick page. Their figures (618 L, 48 sols) come from search snippets only.
- I found no published peer-reviewed analysis of Watney's potato yield arithmetic, and no direct quotes from NASA plant scientists (for example Wheeler or Massa) on the film.
