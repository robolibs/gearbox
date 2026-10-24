# Plant and crop growth models for a real-time Mars-greenhouse robot simulator (gearbox, Rust/Bevy), as of October 2026

Scope: models that grow plants over time in a controlled environment, covering both physiology (biomass, development stage, response to light, CO2, temperature, water and nutrients) and geometry (how the 3D shape changes), with a focus on what can run for thousands of plants per sim tick inside gearbox (Bevy 0.19 in the workspace `Cargo.toml`; Bevy 0.20.0 docs are dated 8 Oct 2026). Research date: 2026-10-23.

---

## 1. Field crop growth models (WOFOST, SUCROS, LINTUL, DSSAT, APSIM, AquaCrop) and NASA's Modified Energy Cascade (MEC): state, inputs, time step, cost, licence, CEA fit, and core equations

### Takeaway
For a real-time simulator the usable cores are the **light-use-efficiency (LUE) family** (LINTUL: intercepted PAR × LUE, Beer's law on LAI, thermal-time phenology, daily Euler step) and NASA's **MEC** (absorbed PPFD × canopy quantum yield × carbon use efficiency; built for closed BLSS chambers with CO2 up to 1300 ppm, run hourly in the Boscheri variant). Both cost a few dozen floating-point operations per plant (or per canopy patch) per step. WOFOST/SUCROS add Gaussian-integrated leaf photosynthesis, which is heavier but still cheap. DSSAT and APSIM are full cropping-system frameworks. They are useful as reference data, not as something to embed in the engine: DSSAT is BSD-3, but APSIM's licence is not open source.

### Cited Findings

**MEC / Energy Cascade (NASA BLSS)**
- The Energy Cascade (EC) model was first calibrated for wheat (Volk et al., 1995), extended to dry bean, lettuce, peanut, white potato, rice, soybean, sweet potato and tomato (Jones & Cavazzoni, 2000), and became the Modified Energy Cascade (MEC) (Cavazzoni 2001, 2004; Cavazzoni 2004 is *Adv. Space Res.* 34:1528–1538). — [Boscheri et al. 2012, Adv. Space Res. 50:941–951](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
- MEC is "explanatory". It relies on multivariate polynomial equations whose coefficients come from ad-hoc curve fitting of experimental data (Cavazzoni 2001). A multi-layer canopy photosynthesis model was used to generate polynomial regressions for CQYmax as a function of irradiance and CO2. Descriptive models add air-temperature effects on the crop parameters. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
- The EC models have five fundamental trends: PPFD absorption rises linearly to canopy closure; canopy quantum yield (CQY) stays constant until senescence and then declines linearly to the end of the life cycle; carbon use efficiency (CUE) is constant over the life cycle for non-legumes, while legumes (soybean, peanut, dry bean) get a CQY-like declining CUE. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
- **MEC core equations** (Cavazzoni 2004 as restated by Boscheri 2012 and Amitrano 2020):
  - Canopy absorption: `A = AMAX·(t/tA)^n` for `t < tA`, and `A = AMAX` for `t ≥ tA`. Here tA is the time of canopy closure (days after emergence) and n is a crop exponent; **n = 2.5 for lettuce**. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf); [Amitrano et al. 2020, Sensors 20:3110](https://pmc.ncbi.nlm.nih.gov/articles/PMC7308940/)
  - Quantum yield: `CQY = CQYMAX` for `t ≤ tQ`, and `CQY = CQYMAX − (CQYMAX − CQYMIN)(t − tQ)/(tM − tQ)` for `tQ < t ≤ tM`. tQ is the onset of senescence and tM is harvest/maturity. CUE24 has the same form for legumes and is constant otherwise. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
  - Daily carbon gain: `DCG = 0.0036 · H · CUE · A · CQY · PPFD` in mol C m⁻² d⁻¹. H is the photoperiod in h, PPFD is in µmol m⁻² s⁻¹, and 0.0036 converts µmol→mol and h→s. Crop growth rate: `CGR = MWc · DCG / BCF` in g m⁻² d⁻¹, with MWc = 12 g mol⁻¹ and BCF the biomass carbon fraction. Edible biomass: `TEB = ∫_{tE}^{tM} XFRT · CGR dt`. Oxygen: `DOP = OPF · DCG`. Gross photosynthesis: `PG = A·CQY·PPFD`. — [Amitrano 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7308940/)
  - Hourly version (Boscheri's MMEC): `HCG = α · CUE24 · A · CQY · PPFD · I`, where I = 1 in light and 0 in dark and α = 0.0036. `HCGR = HCG · MWC / BCF`. Wet growth: `HWCGR = HCGR / (1 − WBF)`. Hourly O2 production: `HOP = HCG/CUE24 · OPF · MWO2`. Respiration O2 use: `HOC = HCG/I · (1 − CUE24)/CUE24 · OPF · MWO2 · H/24`. CO2 use and production are tied stoichiometrically to O2 (Eqs. 14–15). Nutrient uptake: `HNC = HCGR · DRYfr · NCfr`. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
  - Transpiration: `HTR = β · MWw · gC · VPD / Patm` and `gC = gA·gS/(gA + gS)`. For planophile canopies (lettuce, soybean, sweet potato), `gS = (1.717·T − 19.96 − 10.54·VPD) · PNET/[CO2]` with `gA = 2.5` mol m⁻² s⁻¹. For vertical canopies (tomato), `gS = 0.1389 + 15.32·RH · PNET/[CO2]` with `gA = 5.5`. `VPD = VPSAT(1 − RH)` and `VPSAT = 0.611·exp(17.4T/(T + 239))` kPa. `PNET = A·CQY·PPFD`. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
  - Amitrano 2020 gives net photosynthesis as `PN = [H·α/24 + β(24 − H)/24] · PPFD`, with α = CUE·A·CQY and β = A·CQY (quoted as printed; it does not match Boscheri's PNET definition). — [Amitrano 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7308940/)
- **Validity range and time step:** MEC applies over PPFD 200–1000 µmol m⁻² s⁻¹, RH 35–100 % and CO2 330–1300 ppm. The original MEC uses a 1-day step, which Boscheri changed to 1 h to capture day/night transitions and short failures such as a lamp outage. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
- **Accuracy:** In the University of Arizona Lunar Greenhouse prototype (lettuce, tomato, sweet potato and strawberry under HPS lamps, CO2 1000 ppm, about 400 µmol m⁻² s⁻¹ PAR, 17/7 h photoperiod), biomass over the 5-month run was off by −0.7 % and water use by +0.3 %. Shorter windows had errors of 31 % (biomass), 48 % (water) and 51 % (condensate). Lettuce alone came within 9 %. Mixed tall canopies came within 42 % for biomass because per-layer PPFD was poorly estimated. Biomass output was "High" sensitivity to PPFD, "Medium" to CO2, and "None" to air temperature. — [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)
- Lettuce MEC parameters from Amitrano 2020 (Table 2): BCF 0.4; XFRT 0.95; OPF 1.08 mol O2 (mol C)⁻¹; gA 2.5. They calibrated α = CUE·A·CQY (min/max), e.g. green lettuce at nominal VPD 0.007/0.017 and off-nominal VPD 0.003/0.010, with maturity onset tMi = 16 and harvest tM = 23 days after transplanting. Chlorophyll-fluorescence measurements showed that CQY is not constant: it rises, plateaus and then declines, so they modified the MEC. — [Amitrano 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7308940/)
- Three MEC versions now exist (Cavazzoni, Boscheri, Amitrano), and differences in methodology and assumptions change their outputs. — [ASHS 2024 abstract](https://ashs2024.sched.com/event/1dtbx/ce-4-global-sensitivity-analysis-and-validation-of-the-modified-energy-cascade-crop-model-for-controlled-environment-agricult)
- **2026 evaluation (Coon et al., In Silico Plants 8(1), doi 10.1093/insilicoplants/diag011):** All four EC models overestimated lettuce biomass (RMSE 49.58–96.99 g m⁻²) and underestimated transpiration (RMSE 0.80–2.30 L m⁻² d⁻¹) for two cultivars at 21 °C, ambient CO2 and a DLI of 22.03 mol m⁻² d⁻¹. Biomass predictions were insensitive to temperature. The authors attribute the systematic biases to the "erroneous canopy closure methodology and lack of temperature responses." (Abstract only.) — [Coon et al. 2026](https://doi.org/10.1093/insilicoplants/diag011)

**LINTUL family (Wageningen): the simplest usable LUE model**
- LINTUL builds on Monteith's (1977) radiation-use-efficiency concept: dry matter = intercepted light × a constant LUE, partitioned to organs by development stage. In LINTUL-POTATO, SUCROS's photosynthesis and respiration are replaced by a temperature-dependent LUE (Spitters & Schapendonk 1990). — [LINTUL family chapter (BDS)](https://www.bdspublishing.com/webshop/chapters/crops/agri-tech/data-management/the-light-interception-and-utilisation-family-of-crop-models/); [LINTUL-POTATO-DSS, Potato Research 2015](https://link.springer.com/article/10.1007/s11540-015-9303-7)
- **LINTUL3 core equations** (PCSE source, daily step):
  - Intercepted PAR: `PARINT = 0.5 · DTR · (1 − exp(−K · LAI))`, where DTR is daily global radiation and 0.5 is the PAR fraction. Growth: `RGROWTH = LUE · PARINT`, multiplied by `min(TRANRF, NRF)`, where TRANRF = actual/potential transpiration and `NRF = exp(−NLUE·(1 − NNI))`.
  - Thermal time: `DTEFF = max(0, DAVTMP − TBASE)`.
  - Juvenile leaf area (while DVS < 0.2 and LAI < 0.75): `GLAI = LAI·(exp(RGRL·DTEFF·Δt) − 1)/Δt · TRANRF · exp(−NLAI(1 − NNI))`. Otherwise `GLAI = SLA·GLV`, with `SLA = SLAC · SLACF(DVS) · exp(−NSLA(1 − NNI))`.
  - Leaf death: `RDR = max(RDRDV, RDRSH)`, where RDRDV = 0 until TSUM ≥ TSUMAG and `RDRSH = max(0, RDRSHM·(LAI − LAICR)/LAICR)` (self-shading). Extra N-stress death is `WLVG·RDRNS·(1 − NNI)`.
  - Water: potential transpiration is `(1 − exp(−0.5·LAI))·ET0` and soil evaporation is `exp(−0.5·LAI)·ES0`.
  - Nitrogen nutrition index: `NNI = (actual[N] − residual[N])/(critical[N] − residual[N])`, limited to 0.001–1.
  — [PCSE lintul3.py](https://github.com/ajwdewit/pcse/blob/HEAD/pcse/crop/lintul3.py)
- LINTUL3 spring-wheat test parameters: LUE 2.8 g MJ⁻¹, K 0.6, RGRL 0.009 (°C d)⁻¹, SLAC 0.022 m² g⁻¹, TBASE 0 °C, TSUMAG 800 °C d, LAICR 4.0, RDRSHM 0.03 d⁻¹, TSUM1 800 °C d, TSUM2 1030 °C d, initial leaf weight WLVGI 2.4. — [PCSE lintul3_springwheat.crop](https://github.com/ajwdewit/pcse/blob/HEAD/pcse/tests/test_data/lintul3_springwheat.crop)
- LINTUL3 was published for N-limited rice (Shibu et al. 2010, Eur. J. Agron.). N stress acts by reducing LUE and/or leaf area. — [PCSE lintul3.py docstring](https://github.com/ajwdewit/pcse/blob/HEAD/pcse/crop/lintul3.py)
- LINTUL-POTATO-DSS uses a fixed temperature sum for ground cover to develop from 0 % at emergence to 100 %. Its harvest index is 0 for a number of days after emergence, a period that is shorter for earlier varieties, higher temperatures and shorter photoperiods. — [LINTUL-POTATO-DSS 2015](https://link.springer.com/article/10.1007/s11540-015-9303-7) (via search snippets; full text not retrieved)

**WOFOST / SUCROS (via PCSE)**
- PCSE (Python Crop Simulation Environment) contains WOFOST 7.2, 7.3 and 8.1, LINTUL3, LINGRA/LINGRA-N, ALCEPAS and a green-layer crop, with nutrient modules (N and NPK demand, uptake, stress and translocation). Latest tag is 6.0.13, repository pushed 2026-10-19. Licence: EUPL (the LICENSE header says v1.1 "or subsequent"); the WOFOST crop parameter files are EUPL 1.2. — [PCSE GitHub](https://github.com/ajwdewit/pcse); [PCSE LICENSE](https://github.com/ajwdewit/pcse/blob/HEAD/LICENSE); [WOFOST_crop_parameters wofost81](https://github.com/ajwdewit/WOFOST_crop_parameters/blob/wofost81/potato.yaml)
- PCSE's engine uses Euler integration with a fixed 1-day step. Weather inputs are TMAX, TMIN, VAP, WIND, RAIN and IRRAD. Typical outputs are DVS (development stage), LAI, TAGP (total above-ground production), TWSO (storage organs), TWLV, TWST, TWRT, TRA, RD and SM. — [PCSE reference guide](https://pcse.readthedocs.io/en/stable/reference_guide.html)
- WOFOST assimilation follows Spitters et al. (1988): daily gross CO2 assimilation by 3-point Gaussian integration over the day (XGAUSS = 0.1127, 0.5, 0.8873) and over canopy depth, with sunlit/shaded leaf light response `FGROS = AMAX·(1 − exp(−EFF·I/AMAX))`. In 8.x, `AMAX = CO2AMAX · TMPF · min(AMAX_REF, max(0, AMAX_SLP·(SLN − AMAX_LNB)))`, so leaf N enters, and `EFF = EFFTB(T) · CO2EFFTB(CO2)`. — [PCSE assimilation.py](https://github.com/ajwdewit/pcse/blob/HEAD/pcse/crop/assimilation.py)
- **CO2 fertilisation tables (generic C3, WOFOST 8.1):**
  - AMAX multiplier: 0 at 40 ppm, 1.00 at 360, 1.60 at 720, 1.90 at 1000 and 2000.
  - Initial light-use efficiency (EFF) multiplier: 0 at 40, 1.00 at 360, 1.11 at ≥720.
  - Maximum transpiration multiplier: 1.0 up to 360, 0.9 at ≥720.
  - Potato values: TBASEM 3 °C, TSUM1 150 and TSUM2 1550 °C d, RGRLAI 0.012, SPAN 37 d, TBASE 2 °C, AMAX_REF 30 kg CO2 ha⁻¹ h⁻¹, EFF 0.45, KDIF 1.0, SLA 0.003 ha kg⁻¹.
  — [WOFOST potato.yaml](https://github.com/ajwdewit/WOFOST_crop_parameters/blob/wofost81/potato.yaml)
- RUE-based models usually handle CO2 with a normalised response (RUE = 1 at current CO2). The rice study cited here fit a Morgan-Mercer-Flodin curve. EPIC adjusts RUE with crop-specific CO2 response parameters. — [Pesq. Agropec. Bras. rice RUE–CO2 study](https://scielo.br/j/pab/a/MRN8xwn3xFjMjRhxdzqdKBD/?lang=en); [USDA ERS EPIC appendix](https://ers.usda.gov/webdocs/publications/45492/55560_err201_appendicesc-h.pdf?v=581.7)

**AquaCrop (FAO)**
- Canopy cover (CC) growth per day, in either GDD or calendar time: `CC = CC0·exp(CGC·t)` while CC ≤ CCx/2, then `CC = CCx − 0.25·(CCx/CC0)·CCx·exp(−CGC·t)`, capped at CCx. Decline: `CC = CCx·[1 − 0.05·(exp(t·CDC·3.33·((CCx + 2.29)/(CCx0 + 2.29))/(CCx + 2.29)) − 1)]`. — [AquaCrop-OSPy cc_development.py](https://github.com/aquacropos/aquacrop/blob/HEAD/aquacrop/solution/cc_development.py)
- AquaCrop-OSPy is Apache-2.0 and not an official FAO implementation. It mirrors most of FAO AquaCrop 7.1 but lacks salinity and fertility stress, weed management and perennial herbaceous crops. Latest release v3.1.0 (2026-06-22). AquaCrop-OS (Matlab) is also Apache-2.0. — [aquacrop GitHub](https://github.com/aquacropos/aquacrop); [PyPI aquacrop](https://pypi.org/project/aquacrop); [AquaCrop-OS paper](https://ideas.repec.org/a/eee/agiwat/v181y2017icp18-22.html)
- Licence of FAO's own code: a 2021 GMD reviewer said the AquaCrop source "is not published but only an executable file". A PyPI page says the FAO website provides "the official raw source AquaCrop code". The two statements conflict. — [GMD discussion](https://gmd.copernicus.org/articles/14/7309/2021/gmd-14-7309-2021-discussion.html); [PyPI aquacrop](https://pypi.org/project/aquacrop)

**DSSAT and APSIM**
- DSSAT-CSM is BSD 3-Clause and fully open on GitHub since a January 2020 sprint. Latest release v4.8.6.0 (2026-08-12). — [DSSAT open source](https://dssat.net/development/open-source/); [dssat-csm-os GitHub](https://github.com/DSSAT/dssat-csm-os)
- APSIM Next Generation (ApsimX) source is available under a General Use or Special Use Licence. "Use in any way is not permitted unless previously agreed to." GitHub lists the licence as "other" (NOASSERTION). — [ApsimX GitHub](https://github.com/APSIMInitiative/ApsimX)

### Inferences
- **Mars greenhouse choice:** MEC is the only model in this set that was parameterised for closed BLSS chambers. It reports O2, CO2, water and nutrient mass balances, which a Mars habitat sim will need for life-support bookkeeping. However, the 2026 Coon et al. evaluation and the Boscheri sensitivity table both say it ignores temperature for biomass and gets canopy closure wrong. A practical gearbox design is therefore MEC for the gas and water ledger, plus a LINTUL-style or Van Henten-style per-plant state for growth that does respond to temperature.
- **Cost:** A LINTUL/MEC per-plant update is O(1): one `exp` for Beer's law, a few multiplies, and interpolation of a few lookup tables (AFGEN). At 10⁴ plants that is about 10⁵ flops per sim step, negligible next to rendering. WOFOST's Gaussian scheme makes roughly 9 leaf-photosynthesis evaluations per day with exps (3 time points × 3 canopy depths), still cheap. DSSAT and APSIM are Fortran/C# framework codebases and are heavy to embed. Use them offline to generate reference trajectories or to fit simplified models.
- **Mars CO2:** Greenhouse CO2 will likely run above Earth ambient. WOFOST's tables saturate the AMAX factor at 1.9 above 1000 ppm, and MEC's fits stop at 1300 ppm. Clamp to these ranges or flag extrapolation.
- The cleanest crop-agnostic core for gearbox, then, is: `dW/dt = LUE(T, CO2) · f_PAR · (1 − exp(−k·LAI)) · min(stress factors)`, `dTT/dt = max(0, T − Tbase)` with development stage `DVS = TT/TSUM`, LAI from SLA × leaf mass (exponential juvenile phase), and partitioning tables indexed by DVS. This is LINTUL3 reduced to plant scale.

### Gaps
- Cavazzoni's crop-specific MEC tables were not retrieved: the CQYMAX and tA polynomial coefficients in CO2 and PPF, and tQ, tM and CUE for lettuce, potato, wheat, tomato and soybean. They are in Cavazzoni 2001 (Rutgers report to SIMA) and NASA's BVAD (Hanford 2004, JSC 47804; later revisions).
- The SUCROS description itself (Goudriaan & van Laar 1994) was not fetched. SUCROS is characterised here only through its WOFOST and LINTUL descendants.
- No primary statement was found on the exact licence of FAO's official AquaCrop executable or source.
- The ln(CO2) "β-factor" form of the CO2 response was not found in a primary source.

---

## 2. Controlled-environment agriculture (CEA) crop models: lettuce (Van Henten), tomato (TOMGRO, Vanthoor), potato (LINTUL-POTATO, SUBSTOR), plus 2026 lettuce simulators

### Takeaway
**Van Henten's 1994 two-state lettuce model** (structural and non-structural dry weight, driven by temperature, CO2 and PAR) is the standard small CEA model. It is still the base of the newest image-generating lettuce simulators in 2026, such as LettuceVisSim (Wageningen) and an organ-resolved model coupled to Isaac Sim. Tomato has a 5-state reduced TOMGRO and the Vanthoor yield model. Potato is best served by LINTUL-POTATO-style LUE plus thermal time and photoperiod-gated tuber initiation, with SUBSTOR-Potato in DSSAT as the mechanistic reference.

### Cited Findings

**Van Henten (1994) lettuce model**, as restated with parameters by Valábek, Vargan & Paulen, DYCOPS 2025:
- States: structural dry weight x1 and non-structural dry weight x2, both in g m⁻². Inputs: canopy temperature u1 (°C), CO2 (g m⁻³) and PAR (W m⁻²).
- `dx1/dt = rg·x1`
- `dx2/dt = cC·ψp − rg·x1 − ψr − ((1 − cy)/cy)·rg·x1`
- `rg = cg,max · x2/(cg·x1 + x2) · cQ10,g^((T − 20)/10)`
- `ψp = (1 − exp(−ce·ca·(1 − cr)·x1)) · ψp,max`
- `ψp,max = ε·I·G·(C − Γ) / (ε·I + G·(C − Γ))`
- `ε = cε·(C − Γ)/(C + 2Γ)`
- `Γ = cΓ·cQ10,Γ^((T − 20)/10)`
- `1/G = 1/Gb + 1/Gs + 1/Gc`, with `Gc = ac·T² + bc·T + cc` (valid 5–40 °C)
- `ψr = (cs·(1 − cr) + cm·cr)·x1·cQ10,r^((T − 25)/10)`
- Parameters: cC 0.682; cy 0.80; cg 1.20; cε 1.70×10⁻⁵ g J⁻¹; ce 6.75×10⁻²; cr 0.15; ca 1.00; cg,max 5.00×10⁻⁶ s⁻¹; cQ10,Γ 2.0; cQ10,g 1.6; cQ10,r 2.0; cs 3.47×10⁻⁷ s⁻¹; cm 1.16×10⁻⁷ s⁻¹; Gb 7.20×10⁻⁴ m s⁻¹; Gs 5.00×10⁻³ m s⁻¹; ac −1.32×10⁻⁵; bc 5.94×10⁻⁴; cc −2.64×10⁻³; cΓ printed as "4.00×10¹ g m⁻³".
- That study used CasADi at a 1 h sampling time.
— [DYCOPS 2025 paper](https://skoge.folk.ntnu.no/prost/proceedings/dycops-2025/DYCOPS2025_offline_program/contents/papers/0088.pdf)
- Caveats in that restatement: the text names u2 as CO2 and u3 as PAR, but Eq. 5 multiplies ε by u2, so the labels appear swapped. cΓ = 40 "g m⁻³" is physically implausible as a CO2 density. — [DYCOPS 2025](https://skoge.folk.ntnu.no/prost/proceedings/dycops-2025/DYCOPS2025_offline_program/contents/papers/0088.pdf)
- Van Henten-type lettuce models assume abundant water and nutrients and were built for greenhouse climate control. — [search-result snippet, possibly this PMC article](https://pmc.ncbi.nlm.nih.gov/articles/PMC4242352) (attribution uncertain; the page was not opened)

**LettuceVisSim (arXiv 2609.12505, submitted 11 Sep 2026)**
- Authors: Zhu, van 't Ooster, Sun, Eldert van Henten and Boersma; submitted to *Biosystems Engineering*.
- Growth: a process-based model (PBM) derived from Van Henten (1994b), discretised with forward Euler at **Δt = 5 min**, `ŵ(t+1) = ŵ(t) + Δt·g(ŵ, T, I, CO2, plant density, p)`. Five parameters were calibrated (c_b, c_e, c_gr,max, c_K, c_lar). Shoot dry weight fit R² = 0.84.
- Shoot dry weight → projected plant area (PPA), piecewise cubic with a breakpoint at w = 0.7 g per plant:
  - for w < 0.7 g: `PPA = 10⁻³(1.444 + 13.444w − 3.336w² − 1.767w³)` m²
  - for w ≥ 0.7 g: `PPA = 10⁻³(3.789 + 7.206w − 0.592w² + 0.024w³)` m²
  - Fit R² = 0.94.
- Geometry: 31 Blender prefab lettuces, one per leaf count from 3 to 34. Plants start with 3 leaves, gain 1 leaf per day, and cap at 34. Each is scaled uniformly by `r = √(PPA/π)` and placed on a density grid.
- Rendering: Unity produces RGB and segmentation images in under 10 ms, but about 60 ms per frame at 1920×1080 with hourly sampling.
- Canopy coverage matched images at R² = 0.84 from measured dry weight but only 0.40 from PBM-simulated dry weight (0.76 with one outlier excluded). A lighting-control RL policy was trained on its images.
- No code link in the visible text.
— [arXiv abs](https://arxiv.org/abs/2609.12505); [arXiv HTML](https://arxiv.org/html/2609.12505v1)

**Organ-resolved lettuce in Isaac Sim (arXiv 2609.27118, 22 Sep 2026; Rahman et al., Auburn)**
- Each leaf gets local conditions, photosynthesises, pays maintenance respiration, and receives carbon allocated by age, size and local environment.
- Leaf mass, area and age drive a 3D plant in NVIDIA Isaac Sim. Ray tracing computes per-leaf interception and feeds it back to photosynthesis.
- Integration: RK4 at 300 s. Geometry and radiation are updated every 6 h of sim time.
- Geometry: blades of 18×96 patches placed at the 137.5° golden angle. Reference plastochron 53,923.3 s per leaf; blade-area coefficient 0.706; insertion angle 82°→44°.
- Light: diffuse sky sampled over 64 directions, with 2 Monte Carlo bounces.
- Validation: relative RMSE of total dry weight 9.5 %, leaf number 9.2 %, canopy diameter 12.7 %.
- Scenarios: +200 ppm CO2 gave +46.1 % dry weight; ±30 % radiation gave +6.9 %/−10.4 %; interior plants in a 40-plant block had 8.6 % less dry weight than border plants.
- A per-leaf tipburn index (relative area-expansion rate ÷ transpiration latent-heat flux) is reported.
- Only 5 plants per scenario (40-plant block max); per-step compute cost not reported; no public code.
— [arXiv abs](https://arxiv.org/abs/2609.27118); [arXiv HTML](https://arxiv.org/html/2609.27118v1)

**Tomato**
- Original TOMGRO (Jones et al. 1991) has 69 state variables. A reduced TOMGRO (Jones, Kenig & Vallejos 1999) keeps 5 state variables for optimal greenhouse climate control; per a secondary source, these include main-stem node number and leaf area. — [AGRIS: Reduced state-variable tomato growth model](https://agris.fao.org/search/zh/records/65df467363b8185d9cabf51b); [Rev. Mex. Cienc. Agríc. pepper model paper](https://www.scielo.org.mx/pdf/remexca/v2n3/v2n3a7.pdf); [MDPI Agronomy 9(12):873](https://www2.mdpi.com/2073-4395/9/12/873)
- The Vanthoor tomato yield model (Vanthoor, de Visser, Stanghellini & van Henten 2011, *Biosystems Eng.* 110(4):378–395; also chapter 9 of Vanthoor's WUR thesis) is part of a greenhouse climate + yield + economics design method. It has been validated in European and North American greenhouses, and later used unmodified under HPS top-lights and LED inter-lights in Norway. — [WUR record](https://research.wur.nl/en/publications/detailed-description-of-the-tomato-yield-model/); [MDPI Agronomy 9(12):873](https://www2.mdpi.com/2073-4395/9/12/873); [NIBIO](https://nibio.brage.unit.no/nibio-xmlui/handle/11250/2720706)

**Potato**
- SUBSTOR-Potato (in DSSAT) has five developmental stages: pre-planting, sprout elongation, emergence, tuber initiation and maturity. Five genotype parameters are used, with relative temperature functions scaled 0–1 between base, optimum and maximum temperatures. — [DSSAT SUBSTOR-Potato](https://dssat.net/substor-potato)
- Upper critical temperature for tuber initiation (TC) and photoperiod sensitivity (P2) control phenology. — [Phytojournal 2019](https://www.phytojournal.com/archives/2019/vol8issue3/PartBS/8-3-611-400.pdf)
- Field data show tuber-initiation onset hastened curvilinearly with temperature (optimum about 22–24 °C), and delayed by longer photoperiod and lower radiation, multiplicatively. — [search summary of a Seoul National University study](https://link.springer.com/article/10.1007/s11540-015-9303-7) (attribution uncertain; it came from the same result set)
- The WOFOST potato parameter set (thermal sums, AMAX, SLA, CO2 tables) is listed in §1. — [WOFOST potato.yaml](https://github.com/ajwdewit/WOFOST_crop_parameters/blob/wofost81/potato.yaml)

### Inferences
- **Lettuce:** Van Henten is the best fit for gearbox. It has two scalar states per plant, three inputs the greenhouse controller already owns (T, CO2, PAR), rates in s⁻¹ that work at any sub-hour step, and a published mapping from dry weight to projected area and leaf count. The plant-scale (not m²-scale) version needs plant density or per-plant ground area, as LettuceVisSim does.
- **Geometry pipeline:** LettuceVisSim's discrete prefab per leaf count plus continuous uniform scale fits gearbox's existing convention that detailed plants come from packed Poly Haven clumps and not hand-built shader geometry. The Isaac Sim organ-resolved model is the high-fidelity ceiling. Its geometry rebuilds every 6 h and its ray tracing are too heavy for thousands of plants per frame, but the same idea (leaf count from a thermal-time plastochron, golden-angle phyllotaxis, blade size from leaf area) can be done in closed form per plant.
- **Tomato and potato:** Reduced TOMGRO (5 states) or a LINTUL-type LUE model with a DVS-indexed partitioning table is enough to drive fruit set and tuber growth visuals. Full TOMGRO (69 states) and Vanthoor's model fit offline or "hero plant" use better.
- **Mars photoperiod:** Photoperiod-gated tuber initiation (SUBSTOR P2, LINTUL-POTATO-DSS lag) matters if gearbox models Mars sols of 24.6 h or artificial photoperiods. The day length should come from the lighting schedule, not the planet.

### Gaps
- The original Van Henten 1994 thesis and paper were not retrieved, so the DYCOPS restatement could not be cross-checked. In particular cΓ and the input labels need checking, and LettuceVisSim's calibrated values are in an appendix that was not read.
- No source was retrieved giving the Vanthoor model's internal equations (carbohydrate buffer, fruit development stages) or the five reduced-TOMGRO state names in full.
- Coon et al. 2026: only the abstract was available.

---

## 3. Functional-structural plant models (FSPM) and procedural plant geometry: L-systems, GroIMP/XL, OpenAlea, AMAP/GreenLab, space colonisation, ABOP, and growth animation in films and games

### Takeaway
FSPMs couple 3D structure with physiology, but research platforms (GroIMP/XL in Java, L-Py/OpenAlea in Python/C++) are authoring and science tools, not real-time engines. For gearbox, the parts worth borrowing are: **parametric/timed L-systems** (ABOP syntax; differential L-systems for continuous-time growth); **GreenLab's source-sink organ cohorts** at thermal-time growth cycles; and **space colonisation / self-organising** models for branching shrubs. Production growth animation (SpeedTree Cinema, Houdini L-System SOP, and bevy_flora in Rust) works by building the mature structure once and revealing or scaling it by age. bevy_flora's vertex-shader version cut 200 growing trees from 17.6 ms to 0.81 ms of CPU per frame.

### Cited Findings
- **FSPM definition and review:** Vos, Evers, Buck-Sorlin, Andrieu, Chelle & de Visser (2010), "Functional-structural plant modelling: a new versatile tool in crop science", *J. Exp. Bot.* 61(8):2101–2115. FSPMs combine 3D plant structure with selected physiological functions to capture how plants adjust to environment and management. — [catalogue record](https://katalog.dhi-paris.fr/vufind/Record/NLM193618036)
- **GroIMP / XL:** XL is a Java extension for relational growth grammars (RGG), a variant of parallel graph grammars that unifies L-systems and graph grammars. GroIMP is open source. — [GroIMP RGG manual](https://manual.grogra.de/RGG/ch01.html); [Hemmerling et al., Funct. Plant Biol.](https://publish.csiro.au/FP/FP08052). GroIMP releases on GitLab: 3_0_0 (2026-09-22) and 2_2 (2025-05-26); project active 2026-10-20; no licence field in GitLab metadata. — [gitlab.com/grogra/groimp](https://gitlab.com/grogra/groimp)
- **L-Py (OpenAlea):** Mixes L-system construction with Python. Distributed under CeCILL (GPL-compatible). Latest release v3.15.4 (2026-02-11). — [openalea/lpy](https://github.com/openalea/lpy)
- **GreenLab (de Reffye, Kang et al.; AMAP lineage):** A dual-scale automaton simulates organogenesis in growth cycles of constant thermal time. Biomass is computed from transpiration and distributed among expanding organs by relative sink demand. Sink strengths come from allometry and potential growth kinetics, fitted by inverse modelling. — [Yan et al. 2004, Ann. Bot. (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC4242319); [Kang & de Reffye 2007](https://library.wur.nl/ojs/index.php/frontis/article/view/1372). A single source-sink balance variable can control branching and fruit load (Mathieu et al. 2009). — [AGRIS](https://agris.fao.org/search/en/records/65deae387c7033e84beb687a). A 2025 chapter describes GreenLab reconstructing 3D architecture from a 2D planar structure with per-cohort biomass demand. — [HAL INRAE 2025](https://hal.inrae.fr/hal-05107301v1)
- **Differential / timed L-systems:** Prusinkiewicz, Hammel & Mjolsness, "Animation of Plant Development" (SIGGRAPH '93, pp. 351–360), combine L-system productions (discrete events: module creation and removal) with differential equations (continuous growth between events), giving time-lapse-like animation. — [algorithmicbotany.org](https://algorithmicbotany.org/papers/animdev.sig93.html); [SIGGRAPH award citation](https://history.siggraph.org/?p=35586)
- **Space colonisation (Runions, Lane & Prusinkiewicz 2007):**
  - The crown envelope is seeded with N attraction points ("usually hundreds or thousands").
  - Each iteration, each point influences its closest node if it is within the radius of influence d_i. A node v with influence set S(v) grows a new node `v' = v + D·n̂`, where `n̂ = n/‖n‖` and `n = Σ_{s∈S(v)} (s − v)/‖s − v‖`. The direction can be biased by a tropism vector g.
  - Points within the kill distance d_k are removed. Growth terminates when points run out or none are in range.
  - Branch radii follow the pipe model `r^n = r1^n + r2^n` with n usually 2–3.
  - Generating one tree took "a few seconds to a few minutes" on a 3 GHz desktop in 2007.
  — [Runions et al. 2007 PDF](https://www.algorithmicbotany.org/papers/colonization.egwnp2007.pdf)
- **Self-organising trees (Palubicki et al. 2009, ACM TOG 28(3):58):** Tree form emerges from buds and branches competing for light or space, regulated by internal signalling. Users steer it with brushes, sketching, pruning and bending. — [algorithmicbotany.org](https://algorithmicbotany.org/papers/selforg.sig2009.html)
- **SpeedTree growth animation:** Added in SpeedTree v7 Cinema (Nov 2013) only, from work with ILM on *Noah*. A Timeline bar enables growth and sets a baseline speed, with per-generator speed scales. Large trees preview slowly, so a 'Focus' tool limits computation to one branch. — [SpeedTree docs: growth](https://docs.speedtree.com/doku.php?id=growth); [CGW 2013](https://www.cgw.com/Press-Center/News/2013/SpeedTree-v7-Released-with-Sub-D-Growth-Animatio.aspx); [Unity SpeedTree Modeler timeline](https://docs.unity3d.com/speedtree-modeler/manual/timeline-bar.html)
- **Houdini L-System SOP:** Fractional "Generations" give smooth in-between growth when Continuous Angles/Length/Width are on (the last generation is scaled by the fraction). Variables `t` (iteration count) and `g` (age of the current rule) are available, and `chan()` reads keyframed spare parameters. It supports parametric (up to 5 variables per symbol), context-sensitive and stochastic rules. — [SideFX docs](https://www.sidefx.com/docs/houdini/nodes/sop/lsystem.html)
- **bevy_flora (Rust/Bevy):**
  - Usage: a `TreeAge` component plus `Growing { rate }` ages the tree every frame; the plugin rebuilds wood and leaf child meshes when age or params change.
  - CPU rebuild costs 0.1–0.4 ms per tree per frame. With `GpuGrowth` the mature tree is built once and "grown in the vertex shader" from a few KB of per-node state per frame.
  - Benchmark, 200 growing trees (Apple M-series, release build): oak 17.6 ms → 0.81 ms; willow 25.4 → 0.90 ms.
  - Limits: GpuGrowth needs storage buffers in the vertex stage (no WebGL2), and picking, physics and culling use the mature mesh. `BuildInBackground` builds on the async compute pool.
  - The README flags it as "Vibe Coded … no line-by-line human audit".
  — [bevy_flora README](https://github.com/Ladvien/bevy_flora)

### Inferences
- **Recommended geometry strategy for gearbox:** build or select each plant's *mature* topology once (prefab clump, L-system derivation, or space colonisation), then drive what is shown with a per-plant age or development-stage scalar from the crop model. That scalar can feed node reveal, internode and leaf scale, leaf count, and colour, either in a vertex shader (bevy_flora's GpuGrowth pattern) or by swapping discrete prefabs (LettuceVisSim pattern). This avoids per-frame mesh rebuilds, the dominant cost in every system surveyed.
- Differential L-systems are the formal match for coupling crop ODEs to structure: the ODE runs between discrete organ-creation events, and creation thresholds can be thermal-time plastochrons. A GreenLab-style cohort scheme (all organs born in the same growth cycle share state) keeps per-plant state small, roughly O(number of cycles) rather than O(number of organs).
- Space colonisation and self-organising models are offline-speed (seconds per tree in 2007) and better suited to generating variant libraries than to per-tick growth.

### Gaps
- No primary source was found on how shipped games animate crop growth beyond stage swaps (see §5 on Farming Simulator). No GDC talk or article on procedural plant growth in a shipped game was retrieved.
- The free availability and URL of *The Algorithmic Beauty of Plants* PDF was not checked. Its syntax is referenced through the symbios and Houdini docs.
- AMAP software (AmapSim/Xplo) licensing and status in 2026 was not checked.

---

## 4. Rust ecosystem: crates for L-systems, procedural plants and trees, crop models and ODE integration usable with Bevy

### Takeaway
As of October 2026 there are **no Rust ports of WOFOST, LINTUL, Van Henten or MEC** on crates.io. The only crop-model crate is an AGPL Cotton2K port with 28 downloads, so gearbox would implement the small models itself, which is easy given §1–2. For structure there is now an active **symbios** stack: ABOP-compliant parametric, context-sensitive L-systems with a 3D turtle and a Bevy mesh and Avian collider bridge on Bevy 0.19, the same version as gearbox. **bevy_flora** grows trees over time but targets Bevy 0.20. For ODEs, `ode_solvers` is the established general crate and `diffsol` the most capable. A hand-written fixed-step Euler or RK4 over ECS columns is what the published plant models actually use.

### Cited Findings
Data from the crates.io API, queried 2026-10-23 (version, last update, total downloads, licence, Bevy dependency):

| Crate | Version | Updated | Downloads | Licence | Notes |
|---|---|---|---|---|---|
| `symbios` | 1.5.1 | 2026-09-06 | 1,333 | MIT | "Derivation engine for L-Systems (ABOP compliant)"; parametric and context-sensitive `(k,l)`-systems, guards like `A(x) : x > 5 -> …`, seedable `rand_pcg`, structure-of-arrays layout, no `unsafe`, GA mutation/crossover toolkit — [repo](https://github.com/TheJanusStream/symbios) |
| `symbios-turtle-3d` | 0.6.2 | 2026-09-06 | 1,015 | — | 3D turtle → skeleton using glam — [crates.io](https://crates.io/crates/symbios-turtle-3d) |
| `bevy_symbios` | 0.11.0 | 2026-10-07 | 914 | MIT | Depends on bevy ^0.19, symbios ^1.5; L-system skeleton → tube meshes, material groups, optional Avian3D compound capsule colliders, egui palette editor — [repo](https://github.com/TheJanusStream/bevy_symbios) |
| `bevy_flora` | 0.46.0 | 2026-10-14 | 868 | MIT/Apache-2.0 | bevy ^0.20; 20 presets, grows over time, GpuGrowth, seasons, Poisson-disc scatter; created 2026-10-10 (46 versions in 4 days); flagged "Vibe Coded" — [repo](https://github.com/Ladvien/bevy_flora) |
| `bevy_procedural_tree` | 0.4.0 | 2026-07-09 | 2,367 | MIT/Apache-2.0 | bevy ^0.19; port of ez-tree; "Implement growing" is listed as a ToDo — [repo](https://github.com/Affinator/bevy_procedural_tree) |
| `vanaspati` | 1.0.0 | 2026-03-30 | 222 | **GPL-3.0-only** | "Botany and plant behavior engine": logistic growth curves with stages, C3/C4/CAM photosynthesis, Beer–Lambert canopy, Ball-Berry stomata, GDD and chilling-hours phenology, soil water and N pools, organ biomass pools; single release — [repo](https://github.com/MacCracken/vanaspati) |
| `cotton2k` | 0.1.0 | 2026-05-17 | 28 | AGPL-3.0+ | Rust port of the Cotton2K crop model — [crates.io](https://crates.io/crates/cotton2k) |
| `m-system` | 0.1.1 | 2026-05-09 | 50 | MIT | "L-system modeling crate for developmental biology simulations" — [crates.io](https://crates.io/crates/m-system) |
| `sequoia` | 0.1.0 | 2026-06-04 | 49 | Apache-2.0 | bevy ^0.18.1 "beautiful tree generator" — [crates.io](https://crates.io/crates/sequoia) |
| `rusty-systems` | 5.0.0 | 2024-06-30 | 7,374 | MIT | stochastic, context-free; early development — [docs.rs](https://docs.rs/rusty-systems/) |
| `dcc-lsystem` | 0.6.3 | 2021-05-17 | 16,039 | — | builder plus 2D rendering; unmaintained since 2021 — [crates.io](https://crates.io/crates/dcc-lsystem) |
| `hyperion` | 0.2.1 | 2024-12-15 | 2,475 | — | generic L-system — [crates.io](https://crates.io/crates/hyperion) |
| `anabaena` | 0.7.0 | 2023-02-13 | 10,989 | BSD-3 | L-system framework — [crates.io](https://crates.io/crates/anabaena) |
| `lindenmayer` | 0.3.0 | 2023-02-26 | 4,498 | MIT | memory-efficient — [crates.io](https://crates.io/crates/lindenmayer) |
| `lsystem` | 0.2.1 | 2015-12-11 | 27,089 | — | old — [crates.io](https://crates.io/crates/lsystem) |
| `bevy_procedural_vegetation` | 0.1.0 | 2024-04-01 | 1,607 | MIT/Apache-2.0 | bevy ^0.13 (stale) — [crates.io](https://crates.io/crates/bevy_procedural_vegetation) |
| `bevy_grass` | 0.20.5 | 2026-10-22 | 38 | MIT | bevy ^0.20 grass that sways and bends — [crates.io](https://crates.io/crates/bevy_grass) |
| `ode_solvers` | 0.6.2 | 2026-06-07 | 1,243,954 | — | general explicit ODE solvers — [repo](https://github.com/srenevey/ode-solvers) |
| `diffsol` | 0.18.0 | 2026-10-20 | 118,186 | — | ODEs and semi-explicit DAEs, Rust or DiffSL DSL with Enzyme AD and LLVM/Cranelift JIT; JOSS paper 24 Jan 2026 — [repo](https://github.com/martinjrobins/diffsol); [JOSS](https://www.theoj.org/joss-papers/joss.09384/10.21105.joss.09384.pdf) |
| `russell_ode` | 4.0.0 | 2026-10-21 | 30,759 | — | ODE/DAE solvers — [repo](https://github.com/cpmech/russell) |
| `differential-equations` | 0.7.0 | 2026-10-11 | 25,860 | — | [repo](https://github.com/Ryan-D-Gast/differential-equations) |

- crates.io searches for "wofost", "crop model", "crop growth", "greenhouse" and "space colonization" returned no relevant crop-model or colonisation crates beyond `cotton2k` and `vanaspati`. — [crates.io search API](https://crates.io/api/v1/crates?q=wofost)
- The workspace pins `bevy = "0.19"` (gearbox `Cargo.toml`, line 34). Bevy 0.20.0 docs are dated 2026-10-08. — [docs.rs bevy Virtual](https://docs.rs/bevy/latest/bevy/time/struct.Virtual.html)

### Inferences
- **Licences:** `vanaspati` (GPL-3.0-only) and `cotton2k` (AGPL) carry copyleft obligations. Read them for ideas but do not link them into gearbox unless gearbox's licence allows it.
- `bevy_symbios` matches gearbox's Bevy 0.19 today and adds Avian colliders. `bevy_flora` would need a Bevy 0.20 upgrade, or using only its techniques (vertex-shader growth). Given the "vibe coded" flag and 46 releases in 4 days, treat bevy_flora as a design reference rather than a dependency.
- **ODE choice:** All the plant models here are small, non-stiff, explicit systems integrated by fixed-step Euler or RK4 (PCSE daily Euler; LettuceVisSim 5-min Euler; organ-resolved model RK4 at 300 s). Per-plant adaptive solvers (diffsol, russell) are overkill and branchy when vectorised. A bespoke SoA Euler or RK4 loop over ECS component columns is the efficient choice. `ode_solvers` or `diffsol` are useful offline for verification against reference runs.

### Gaps
- Licence fields for some crates (dcc-lsystem, hyperion, ode_solvers, diffsol, symbios-turtle-3d) were not printed by the query used and should be checked on crates.io.
- No Bevy showcase projects that grow *crops* over time (as opposed to trees) were found. The Bevy community showcase was not searched directly.

---

## 5. Accelerated time, batched per-plant updates, instancing thousands of plants, and exposing plant state to robots (NDVI, canopy height, leaf area)

### Takeaway
Bevy provides the needed clock primitives: `Time<Virtual>` with `set_relative_speed`, `max_delta` and `pause`, driving `Time<Fixed>` (64 Hz default, 0..n `FixedUpdate` runs per frame). The growth tick should be decoupled from both and run on simulated plant time, at minutes to hours per step, as the published models do. Rendering thousands of plants depends on not rebuilding meshes: Bevy auto-instances entities that share a mesh and material and has GPU-driven rendering since 0.16. Robot-facing observables can be derived from model state in closed form: canopy cover from LAI by Beer's law, NDVI from LAI by Baret–Guyot, height and diameter from allometry, and images and segmentation as in LettuceVisSim.

### Cited Findings
- `Time<Virtual>` can be paused (delta becomes 0) and `set_relative_speed(2.0)` makes it advance twice as fast as real time. `max_delta` caps virtual time added per update to avoid a "death spiral" after stalls. — [docs.rs bevy 0.20 Virtual](https://docs.rs/bevy/latest/bevy/time/struct.Virtual.html)
- `Time<Fixed>` defaults to 64 Hz (15,625 µs; 60 Hz was avoided because it can alternate between 0 and 2 steps per frame). It follows virtual time, so pause, speed and max_delta apply. `FixedUpdate` "may run 0, 1 or more times during a single update", based on accumulated overstep. — [docs.rs bevy 0.20 Fixed](https://docs.rs/bevy/latest/bevy/time/struct.Fixed.html)
- Bevy 0.16 (24 Apr 2025) extended GPU-driven rendering to most standard 3D meshes, including skinned meshes, enabled automatically on supported platforms. — [Bevy 0.16 release notes](https://bevy.org/news/bevy-0-16/). Bevy batches and instances meshes automatically when they share mesh and material handles. — [Bevy custom shader instancing example](https://bevy.org/examples-webgpu/shaders/custom-shader-instancing/)
- bevy_flora's cost figures again: a CPU mesh rebuild is 0.1–0.4 ms per growing tree per frame, versus about 0.004 ms per tree with GpuGrowth (0.81 ms for 200 oaks). Shared-material recolouring is done in the shader because re-preparing materials "would cost the CPU 60 ms a frame for 20 000 trees". — [bevy_flora README](https://github.com/Ladvien/bevy_flora)
- **Plant-model time steps:** PCSE uses a fixed 1-day Euler step ([PCSE](https://pcse.readthedocs.io/en/stable/reference_guide.html)). MMEC uses 1 h ([Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)). LettuceVisSim uses a 5-min Euler step with image renders hourly or daily; hourly rendering was about 22× slower than daily ([LettuceVisSim](https://arxiv.org/html/2609.12505v1)). The organ-resolved model uses RK4 at 300 s with geometry every 6 h ([arXiv 2609.27118](https://arxiv.org/html/2609.27118v1)).
- **Game time scaling (Farming Simulator 22):** 4 seasons with a default of 3 in-game days per season (adjustable up to 28). Growth periods adapt to days per month, so changing that setting does not speed growth. Seasonal growth can be disabled. — [Farming Simulator wiki](https://farmingsimulator.wiki.gg/wiki/Seasons/Farming_Simulator_22). Per FS19 community posts, each crop has a timer between growth stages, so a field sown late in a timer can lag (user explanation, not official). — [Steam FS19 discussion](https://steamcommunity.com/app/787860/discussions/0/2564160288796540415)
- **Robot-sim precedent:** CropCraft (Romea; Apache-2.0) procedurally generates crop-field 3D models via Blender from YAML, for real-time robotics simulation. It exports to Gazebo Classic with a per-crop description file and annotated images, and provides ground truth for plant types in LiDAR. — [CropCraft GitHub](https://github.com/Romea/cropcraft); [arXiv 2511.02417](https://arxiv.org/pdf/2511.02417). A ROS project generated tomato meshes in Blender and assembled Ignition Gazebo worlds with plant and fruit ground-truth positions. — [ROS Discourse](https://discourse.ros.org/t/randomly-generated-ignition-gazebo-crop-fields-for-agriculture-robot-testing/17268)
- **NDVI–LAI:** Following Baret & Guyot (1991, *Remote Sens. Environ.* 35:161–173), a vegetation index saturates with LAI as a Beer-like exponential: `VI = VI∞ + (VIg − VI∞)·exp(−K_VI·LAI)`, where VIg is bare soil, VI∞ the asymptote and K_VI an extinction-like coefficient. VI∞ and K_VI depend on leaf angle distribution, soil reflectance and sun/view angles. — [HAL record](https://hal.inrae.fr/hal-02701349); [TIB-hosted ISPRS proceedings](https://goobi.tib.eu/viewer/fulltext/856910465/482); [Saito et al., ACRS 2001](https://a-a-r-s.org/proceeding/ACRS2001/PapersF/155SAITO.pdf)
- **Physically based spectra:** PROSPECT models leaf reflectance and transmittance (400–2450 nm) from chlorophyll a/b, carotenoids, water and dry matter. PROSAIL couples it with the SAIL canopy model using LAI, leaf inclination and sun angle, so canopy spectra and indices can be simulated from model state. — [PROSPECT-MP, Sci. Rep. 2017 (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC5526878); [PROSAIL parameter table, Plant Methods](https://plantmethods.biomedcentral.com/articles/10.1186/s13007-020-00643-z/tables/3)
- LettuceVisSim validated its canopy-layout ground-coverage ratio against 12 experimental image datasets and outputs RGB plus binary segmentation masks from a top-down orthographic camera. — [LettuceVisSim](https://arxiv.org/html/2609.12505v1)

### Inferences
- **Clock design for gearbox** (consistent with its fixed 120 Hz physics noted in project memory):
  - Keep physics on its fixed step. Run a separate plant-growth accumulator on sim time scaled by a "growth speed" factor, which could be thousands× for days-per-minute playback.
  - When the accumulator crosses the plant step (e.g. 5 min or 1 h sim time), run k substeps over all plants in one batched SoA system and cap k per frame (amortise across frames or chunks) so time-lapse does not stall rendering.
  - Because the models are explicit and smooth, a step of 1 h or more is acceptable when sped up. Switch to 5-min steps when a robot is interacting in real time.
- **Batching:** store per-plant states as ECS components (e.g. `CropState { w_struct, w_nonstruct, tt, lai, stress }`) and update them in one parallel query. Group plants by tray or bed sharing the same climate inputs, so that T, CO2, PAR and VPD are computed once per zone.
- **Rendering:** many plants share one mesh per species × growth-stage bucket (auto-instanced). Continuous variation (scale, colour, wilting angle) goes through per-instance data or vertex-shader parameters, not unique meshes. This matches bevy_flora's measurements and gearbox's existing GPU-sown grass approach.
- **Sensors for robots, cheap and derived from state:**
  - Canopy cover: `f = 1 − exp(−k·LAI)`.
  - NDVI from the Baret–Guyot form, plus noise.
  - Canopy height and diameter from allometry on dry weight or leaf count; LettuceVisSim's projected area gives a diameter of 2·√(PPA/π).
  - Leaf count from the plastochron.
  - Fresh mass = dry mass/(1 − water fraction), as in MEC's WBF.
  - Stress indices (TRANRF, NNI, VPD-off-nominal factor) exposed only through proxies: colour, NDVI drop, leaf temperature.
  - Physically rendered multispectral cameras could sample PROSPECT-derived albedo per band. That is heavier and optional.

### Gaps
- No primary game-developer source describing batched per-plant updates or time-lapse growth in a shipped title was found.
- Bevy's default `Time<Virtual>` max_delta value was not confirmed from the fetched page.
- A concrete PROSAIL-based NDVI pipeline in a real-time renderer was not found.

---

## 6. Modelling visible plant stress (wilting, yellowing/chlorosis, stunting) from model state

### Takeaway
The crop models already compute dimensionless stress factors that can drive visuals directly. LINTUL3's water-stress factor TRANRF and nitrogen nutrition index NNI reduce LUE, SLA and leaf area, and accelerate leaf death. The MEC/Amitrano off-nominal VPD calibrations cut α by about 40–60 %. The organ-resolved lettuce model adds a tipburn index. Graphics research supplies the mapping from stress to shape: wilting from water content mapped to stiffness, solved with position-based dynamics (SIGGRAPH Asia 2023), or real-time mass-spring leaves with vein and mesophyll springs. Leaf colour can be physically grounded with PROSPECT (chlorophyll → reflectance).

### Cited Findings
- **Stunting and N stress in LINTUL3:** growth is multiplied by `min(TRANRF, exp(−NLUE·(1 − NNI)))`. Specific leaf area becomes `SLA = SLAC·SLACF(DVS)·exp(−NSLA·(1 − NNI))`. Juvenile LAI growth is scaled by `TRANRF·exp(−NLAI·(1 − NNI))`. Partitioning shifts to roots under water stress (`FRTMOD = max(1, 1/(TRANRF + 0.5))`) and away from leaves under N stress (`exp(−NPART·(1 − NNI))`). Extra leaf death is `WLVG·RDRNS·(1 − NNI)`. — [PCSE lintul3.py](https://github.com/ajwdewit/pcse/blob/HEAD/pcse/crop/lintul3.py)
- **Water and VPD stress in MEC lettuce:** Amitrano's calibrated α range for green lettuce fell from 0.007–0.017 (nominal VPD) to 0.003–0.010 (off-nominal VPD), and similarly for red lettuce (0.007–0.021 → 0.003–0.011). — [Amitrano 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7308940/)
- **Tipburn:** the organ-resolved lettuce model computes per-leaf tipburn as positive relative area-expansion rate ÷ transpiration latent-heat flux (m² J⁻¹). In a 40-plant block, the index rose in enclosed leaves. — [arXiv 2609.27118](https://arxiv.org/html/2609.27118v1)
- **Wilting (graphics):** Maggioli et al. (ACM TOG, SIGGRAPH Asia 2023) model water transport inside the plant, map each part's water content to branch stiffness, and solve the wilted shape with position-based dynamics. They report a closer match to measured wilting than models without internal water flow. — [project page](https://gladia.di.uniroma1.it/publication/maggioli-2023-plant/); [PDF](https://wp.faculty.wmi.amu.edu.pl/papers/Maggioli.etal-2023-PlantWilting.pdf). Silva et al. couple a leaf mass-spring mesh with fluid flow and evaporation, so cell water content (turgor) drives shrinking and curling. — [IWAIT 2013 PDF](https://graphics.cmlab.csie.ntu.edu.tw/~robin/docs/iwait13.pdf). A venation-based mass-spring method assigns different contraction forces to vein and mesophyll springs and reports real-time speed. — [JCAD](https://jcad.cn/article/id/78918ebd-74cf-474f-a8f0-ac1ee97fd095)
- **Leaf colour:** PROSPECT separates chlorophyll a, chlorophyll b and carotenoid effects on 400–800 nm reflectance (PROSPECT-MP); the a/b ratio shifts with leaf age and stress. — [PROSPECT-MP (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC5526878)
- **Seasonal colour in Bevy practice:** bevy_flora drives autumn colour, leaf fall and blossom from a season value. It blends colour on the GPU through a shared buffer rather than rewriting materials. — [bevy_flora README](https://github.com/Ladvien/bevy_flora)

### Inferences
- **Suggested state → visual mapping**, all cheap per instance:
  - *Stunting:* falls out of the growth model itself. Lower dry weight leads to smaller projected area, fewer leaves (if the plastochron is carbon-limited, as in the organ-resolved model) and a smaller prefab or scale.
  - *Wilting:* track a fast water-status scalar ψ (e.g. a 1-hour exponential filter of TRANRF or a supply/demand ratio) separate from slow biomass. Map it to leaf droop angle and petiole bend in the vertex shader (a cheap stand-in for Maggioli's stiffness → PBD), and recover when ψ returns, as turgor does. Persistent ψ deficit then feeds TRANRF into growth.
  - *Chlorosis:* drive a chlorophyll proxy from NNI (or MEC's nutrient deficit). Lerp leaf albedo from a healthy green to a yellow derived from PROSPECT reflectance at low Cab, older leaves first (N is remobilised from old leaves). Use the same Cab value to lower simulated NDVI and red-edge readings, so that camera colour and sensor values stay consistent.
  - *Senescence and necrosis:* use LINTUL's leaf-death terms (RDRDV after TSUMAG, RDRSH above LAICR) to brown or remove the oldest leaves. Use the tipburn index to darken leaf margins on inner lettuce leaves.
- Exaggerate stress visuals first when tuning, then calibrate against photos. This follows the project's existing "exaggerate before tuning" verification habit.

### Gaps
- No source was found giving a quantitative mapping from NNI or leaf N to visible leaf colour (RGB) or SPAD values usable directly in a renderer. PROSPECT gives spectra, but the conversion to sRGB under the greenhouse's LED spectrum would need to be built.
- No real-time-validated, model-coupled wilting implementation in a game engine was found. The graphics papers above are offline or standalone.
- Effects of low total pressure (hypobaric Mars greenhouse) on growth and transpiration were out of scope and not researched. MEC's transpiration does include Patm explicitly (`HTR ∝ gC·VPD/Patm`; [Boscheri 2012](https://simoc.space/wp-content/uploads/2018/09/Boscheri-et-al-2012.pdf)).
