%% generate_corpus.m — regenerate the validator's oracle corpus.
%
% For every family below this script builds a small Pulseq 1.5.1 sequence with
% KNOWN generation parameters, writes the `.seq`, a `params.json` sidecar (the
% known inputs — the "recover the inputs" ground truth), the full `testReport()`
% text (provenance), and a `report.json` with the scalars testReport measures
% independently (TE / TR / duration — the second, independent oracle).
%
% The committed artifacts under corpus/data/ are what CI checks the Rust
% validator against; MATLAB is only needed to (re)generate them. Run headless:
%
%   matlab -batch "run('corpus/matlab/generate_corpus.m')"
%
% Requirements: the Pulseq mr-toolbox on the MATLAB path — set the PULSEQ_MATLAB
% env var to its `matlab` directory, or addpath() it yourself before running. The
% bundled fnint.m shim lets testReport's k-space integration run without the
% Curve Fitting Toolbox.

clear; clc;
here = fileparts(mfilename('fullpath'));
addpath(here);                                   % fnint.m shim (first, so it is found)
pulseq_matlab = getenv('PULSEQ_MATLAB');
if ~isempty(pulseq_matlab)
    addpath(pulseq_matlab);
end
if isempty(which('mr.Sequence'))
    error('generate_corpus:noPulseq', ...
        ['Pulseq mr-toolbox not found. Set the PULSEQ_MATLAB env var to its ' ...
         '`matlab` directory (e.g. export PULSEQ_MATLAB=/path/to/pulseq/matlab) ' ...
         'or addpath() it before running this script.']);
end
outdir = fullfile(here, '..', 'data');
if ~exist(outdir, 'dir'); mkdir(outdir); end

[vm, vn, vr] = mr.aux.version('output');
fprintf('mr-toolbox version %d.%d.%d\n', vm, vn, vr);

sys = mr.opts('MaxGrad', 30, 'GradUnit', 'mT/m', 'MaxSlew', 150, 'SlewUnit', 'T/m/s', ...
    'rfRingdownTime', 20e-6, 'rfDeadTime', 100e-6, 'adcDeadTime', 10e-6);

% family name -> builder thunk. Each returns (seq, params-struct).
families = {
    'gre2d_1slice',  @() build_gre(sys, struct('Nslices',1,'rf_spoil',false,'flip',15,'TR',50e-3,'TE',10e-3))
    'gre2d_3slice',  @() build_gre(sys, struct('Nslices',3,'rf_spoil',false,'flip',15,'TR',50e-3,'TE',10e-3))
    'spgr2d_1slice', @() build_gre(sys, struct('Nslices',1,'rf_spoil',true, 'flip',10,'TR',12e-3,'TE',5e-3))
    'gre3d_8part',   @() build_gre3d(sys)
    'mgre2d_1slice', @() build_mgre(sys)
    'epi2d_1slice',  @() build_epi(sys, 1)
    'epi2d_3slice',  @() build_epi(sys, 3)
    'se2d_1slice',   @() build_se(sys)
};

nfam = size(families, 1);
nok = 0;
for f = 1:nfam
    name = families{f, 1};
    try
        [seq, p] = families{f, 2}();
        [ok, err] = seq.checkTiming();
        if ~ok
            fprintf('[FAIL] %-14s checkTiming failed:\n%s\n', name, strjoin(err, ''));
            continue;
        end
        % Capture and strip the oracle-comparability flags before params.json is
        % written, so the sidecar holds only the known generation inputs.
        cmp_te = ~isfield(p, 'cmp_oracle_te') || p.cmp_oracle_te;
        cmp_tr = ~isfield(p, 'cmp_oracle_tr') || p.cmp_oracle_tr;
        for fn = {'cmp_oracle_te', 'cmp_oracle_tr'}
            if isfield(p, fn{1}); p = rmfield(p, fn{1}); end
        end

        p.name = name;
        p.scan_time_s = seq.duration();
        seq.setDefinition('Name', name);

        seqpath = fullfile(outdir, [name '.seq']);
        seq.write(seqpath);
        write_json(fullfile(outdir, [name '.params.json']), p);

        rep = seq.testReport();
        s = strjoin(rep, '');
        fid = fopen(fullfile(outdir, [name '.testreport.txt']), 'w');
        fprintf(fid, '%s', s); fclose(fid);

        % Independent oracle = the scalars testReport measures from k-space. Some
        % are not comparable to our per-metric definition and are suppressed
        % (NaN -> JSON null -> the Rust test skips them): testReport's TE for a
        % multi-gradient-echo at fixed ky picks an ambiguous middle echo, and its
        % TR for a single-shot-per-slice family is the slice interval, not the
        % per-slice TR.
        oracle = struct( ...
            'te_s',        ternary(cmp_te, grab(s, 'TE:\s*([0-9.eE+-]+|NaN)s'), NaN), ...
            'tr_s',        ternary(cmp_tr, grab(s, 'TR:\s*([0-9.eE+-]+|NaN)s'), NaN), ...
            'scan_time_s', grab(s, 'Sequence duration:\s*([0-9.eE+-]+|NaN)s'));
        write_json(fullfile(outdir, [name '.report.json']), oracle);

        fprintf('[ ok ] %-14s flip=%g n_slices=%d TR=%.4gs TE=%.4gs dur=%.4gs | report TE=%.4gs TR=%.4gs\n', ...
            name, p.flip_deg, p.n_slices, p.tr_s, p.te_s, p.scan_time_s, oracle.te_s, oracle.tr_s);
        nok = nok + 1;
    catch e
        fprintf('[FAIL] %-14s %s\n', name, e.message);
        if ~isempty(e.stack); fprintf('         at %s:%d\n', e.stack(1).name, e.stack(1).line); end
    end
end
fprintf('\nGenerated %d/%d corpus sequences into %s\n', nok, nfam, outdir);

%% ---------- helpers ----------

function v = grab(s, pat)
    tok = regexp(s, pat, 'tokens', 'once');
    if isempty(tok); v = NaN; else; v = str2double(tok{1}); end
end

function v = ternary(cond, a, b)
    if cond; v = a; else; v = b; end
end

function write_json(path, st)
    fid = fopen(path, 'w');
    fprintf(fid, '%s\n', jsonencode(st, 'PrettyPrint', true));
    fclose(fid);
end

%% ---------- builders ----------

function [seq, p] = build_gre(sys, o)
    % Spoiled 2-D GRE; multi-slice via per-slice RF frequency offset; optional
    % 84-degree-increment RF spoiling (the SPGR variant). One ADC per excitation.
    fov = 200e-3; Nx = 32; Ny = 32; thk = 5e-3;
    seq = mr.Sequence(sys);
    [rf, gz, gzr] = mr.makeSincPulse(o.flip*pi/180, sys, 'Duration', 1e-3, ...
        'SliceThickness', thk, 'apodization', 0.42, 'timeBwProduct', 4, 'use', 'excitation');
    deltak = 1/fov;
    gx = mr.makeTrapezoid('x', sys, 'FlatArea', Nx*deltak, 'FlatTime', 1.6e-3);
    adc = mr.makeAdc(Nx, sys, 'Duration', gx.flatTime, 'Delay', gx.riseTime);
    gxPre = mr.makeTrapezoid('x', sys, 'Area', -gx.area/2, 'Duration', 1e-3);
    gxSpoil = mr.makeTrapezoid('x', sys, 'Area', 2*Nx*deltak);
    phaseAreas = ((0:Ny-1)-Ny/2)*deltak;
    grt = seq.gradRasterTime;
    delayTE = ceil((o.TE - mr.calcDuration(gxPre) - gz.fallTime - gz.flatTime/2 - mr.calcDuration(gx)/2)/grt)*grt;
    delayTR = ceil((o.TR - mr.calcDuration(gz) - mr.calcDuration(gxPre) - mr.calcDuration(gx) - delayTE)/grt)*grt;
    assert(delayTE >= 0 && delayTR >= 0, 'negative TE/TR delay');
    rf_phase = 0; rf_inc = 0;
    for s = 1:o.Nslices
        rf.freqOffset = gz.amplitude*thk*(s-1-(o.Nslices-1)/2);
        for i = 1:Ny
            if o.rf_spoil
                rf.phaseOffset = rf_phase/180*pi; adc.phaseOffset = rf_phase/180*pi;
                rf_inc = mod(rf_inc+84, 360); rf_phase = mod(rf_phase+rf_inc, 360);
            end
            seq.addBlock(rf, gz);
            gyPre = mr.makeTrapezoid('y', sys, 'Area', phaseAreas(i), 'Duration', mr.calcDuration(gxPre));
            seq.addBlock(gxPre, gyPre, gzr);
            seq.addBlock(mr.makeDelay(delayTE));
            seq.addBlock(gx, adc);
            gyRew = mr.makeTrapezoid('y', sys, 'Area', -phaseAreas(i), 'Duration', mr.calcDuration(gxPre));
            seq.addBlock(gyRew, gxSpoil, mr.makeDelay(delayTR));
        end
    end
    seq.setDefinition('FOV', [fov fov thk]);
    p = struct('family','gre2d', 'flip_deg',o.flip, 'n_slices',o.Nslices, ...
        'tr_s',o.TR, 'te_s',o.TE, 'echo_spacing_s',NaN);
end

function [seq, p] = build_gre3d(sys)
    % Non-selective 3-D GRE: a hard block-pulse excitation (single frequency =>
    % one slab => n_slices 1), Cartesian gy/gz partition encoding.
    fov = [192e-3 192e-3 96e-3]; Nx = 16; Ny = 16; Nz = 8; TE = 8e-3; TR = 20e-3; flip = 10;
    seq = mr.Sequence(sys);
    rf = mr.makeBlockPulse(flip*pi/180, sys, 'Duration', 0.2e-3, 'use', 'excitation');
    deltak = 1./fov;
    gx = mr.makeTrapezoid('x', sys, 'FlatArea', Nx*deltak(1), 'FlatTime', 2e-3);
    adc = mr.makeAdc(Nx, sys, 'Duration', gx.flatTime, 'Delay', gx.riseTime);
    gxPre = mr.makeTrapezoid('x', sys, 'Area', -gx.area/2, 'Duration', 1e-3);
    gxSpoil = mr.makeTrapezoid('x', sys, 'Area', gx.area, 'Duration', 1e-3);
    areaY = ((0:Ny-1)-Ny/2)*deltak(2);
    areaZ = ((0:Nz-1)-Nz/2)*deltak(3);
    grt = seq.gradRasterTime;
    delayTE = ceil((TE - mr.calcDuration(rf) + mr.calcRfCenter(rf) + rf.delay - mr.calcDuration(gxPre) - mr.calcDuration(gx)/2)/grt)*grt;
    % The rephase/spoiler gradients share the final block with the TR delay, so
    % the delay (not the spoiler) sets that block's length — do not subtract the
    % spoiler duration here, or the realised TR comes up one spoiler short.
    delayTR = ceil((TR - mr.calcDuration(rf) - mr.calcDuration(gxPre) - mr.calcDuration(gx) - delayTE)/grt)*grt;
    assert(delayTE >= 0 && delayTR >= mr.calcDuration(gxSpoil), 'bad TE/TR delay');
    for iZ = 1:Nz
        gzPre = mr.makeTrapezoid('z', sys, 'Area', areaZ(iZ), 'Duration', 1e-3);
        gzReph = mr.makeTrapezoid('z', sys, 'Area', -areaZ(iZ), 'Duration', 1e-3);
        for iY = 1:Ny
            gyPre = mr.makeTrapezoid('y', sys, 'Area', areaY(iY), 'Duration', 1e-3);
            gyReph = mr.makeTrapezoid('y', sys, 'Area', -areaY(iY), 'Duration', 1e-3);
            seq.addBlock(rf);
            seq.addBlock(gxPre, gyPre, gzPre);
            seq.addBlock(mr.makeDelay(delayTE));
            seq.addBlock(gx, adc);
            seq.addBlock(gyReph, gzReph, gxSpoil, mr.makeDelay(delayTR));
        end
    end
    seq.setDefinition('FOV', fov);
    p = struct('family','gre3d', 'flip_deg',flip, 'n_slices',1, ...
        'tr_s',TR, 'te_s',TE, 'echo_spacing_s',NaN);
end

function [seq, p] = build_mgre(sys)
    % Multi-gradient-echo (bipolar): one excitation, several read-out echoes at a
    % fixed ky. The k-space-centre echo is the first (all echoes share ky), and
    % the echo spacing is the readout-block duration.
    fov = 200e-3; Nx = 32; Ny = 32; thk = 5e-3; TE1 = 5e-3; TR = 100e-3; flip = 20; necho = 4;
    seq = mr.Sequence(sys);
    [rf, gz, gzr] = mr.makeSincPulse(flip*pi/180, sys, 'Duration', 1e-3, ...
        'SliceThickness', thk, 'apodization', 0.42, 'timeBwProduct', 4, 'use', 'excitation');
    deltak = 1/fov;
    gx = mr.makeTrapezoid('x', sys, 'FlatArea', Nx*deltak, 'FlatTime', 1.6e-3);
    adc = mr.makeAdc(Nx, sys, 'Duration', gx.flatTime, 'Delay', gx.riseTime);
    gxPre = mr.makeTrapezoid('x', sys, 'Area', -gx.area/2, 'Duration', 1e-3);
    gxSpoil = mr.makeTrapezoid('x', sys, 'Area', 2*Nx*deltak);
    phaseAreas = ((0:Ny-1)-Ny/2)*deltak;
    esp = mr.calcDuration(gx);
    grt = seq.gradRasterTime;
    delayTE = ceil((TE1 - mr.calcDuration(gxPre) - gz.fallTime - gz.flatTime/2 - mr.calcDuration(gx)/2)/grt)*grt;
    assert(delayTE >= 0, 'negative TE delay');
    for i = 1:Ny
        seq.addBlock(rf, gz);
        gyPre = mr.makeTrapezoid('y', sys, 'Area', phaseAreas(i), 'Duration', mr.calcDuration(gxPre));
        seq.addBlock(gxPre, gyPre, gzr);
        seq.addBlock(mr.makeDelay(delayTE));
        gxe = gx;
        for e = 1:necho
            seq.addBlock(gxe, adc);
            gxe.amplitude = -gxe.amplitude;     % bipolar readout
        end
        gyRew = mr.makeTrapezoid('y', sys, 'Area', -phaseAreas(i), 'Duration', mr.calcDuration(gxPre));
        used = mr.calcDuration(rf, gz) + mr.calcDuration(gxPre) + delayTE + necho*esp;
        delayTR = ceil((TR - used - mr.calcDuration(gxSpoil))/grt)*grt;
        if delayTR < 0; delayTR = 0; end
        seq.addBlock(gyRew, gxSpoil, mr.makeDelay(delayTR));
    end
    seq.setDefinition('FOV', [fov fov thk]);
    % testReport's single TE for a fixed-ky multi-echo train is an ambiguous
    % middle echo; our effective TE is the first (k-centre) echo, so the oracle TE
    % is not comparable here (the generated TE1 is the ground truth instead).
    p = struct('family','mgre2d', 'flip_deg',flip, 'n_slices',1, ...
        'tr_s',NaN, 'te_s',TE1, 'echo_spacing_s',esp, 'cmp_oracle_te',false);
end

function [seq, p] = build_epi(sys, Nslices)
    % Single-shot gradient-echo EPI; multi-slice via per-slice RF frequency
    % offset. An echo train (one ADC per ky line) => effective TE is the central
    % ky echo and the echo spacing is the readout+blip period.
    fov = 220e-3; Nx = 32; Ny = 32; thk = 3e-3;
    seq = mr.Sequence(sys);
    [rf, gz] = mr.makeSincPulse(pi/2, sys, 'Duration', 3e-3, ...
        'SliceThickness', thk, 'apodization', 0.5, 'timeBwProduct', 4, 'use', 'excitation');
    deltak = 1/fov;
    kWidth = Nx*deltak;
    dwell = 4e-6;
    readoutTime = Nx*dwell;
    flatTime = ceil(readoutTime/sys.gradRasterTime)*sys.gradRasterTime;
    gx = mr.makeTrapezoid('x', sys, 'Amplitude', kWidth/readoutTime, 'FlatTime', flatTime);
    adc = mr.makeAdc(Nx, sys, 'Duration', readoutTime, 'Delay', gx.riseTime+flatTime/2-(readoutTime-dwell)/2);
    preTime = 8e-4;
    gxPre = mr.makeTrapezoid('x', sys, 'Area', -gx.area/2, 'Duration', preTime);
    gzReph = mr.makeTrapezoid('z', sys, 'Area', -gz.area/2, 'Duration', preTime);
    gyPre = mr.makeTrapezoid('y', sys, 'Area', -Ny/2*deltak, 'Duration', preTime);
    dur = ceil(2*sqrt(deltak/sys.maxSlew)/10e-6)*10e-6;
    gy = mr.makeTrapezoid('y', sys, 'Area', deltak, 'Duration', dur);
    esp = mr.calcDuration(gx) + mr.calcDuration(gy);
    for s = 1:Nslices
        rf.freqOffset = gz.amplitude*thk*(s-1-(Nslices-1)/2);
        seq.addBlock(rf, gz);
        seq.addBlock(gxPre, gyPre, gzReph);
        gxe = gx;
        for i = 1:Ny
            seq.addBlock(gxe, adc);
            if i < Ny; seq.addBlock(gy); end
            gxe.amplitude = -gxe.amplitude;
        end
    end
    seq.setDefinition('FOV', [fov fov thk]);
    % One excitation per slice => testReport's TR is the slice interval, while our
    % per-slice TR falls back to the whole-scan duration; not comparable, so the
    % oracle TR is suppressed (TE, however, is the central-ky echo and is checked).
    p = struct('family','epi2d', 'flip_deg',90, 'n_slices',Nslices, ...
        'tr_s',NaN, 'te_s',NaN, 'echo_spacing_s',esp, 'cmp_oracle_tr',false);
end

function [seq, p] = build_se(sys)
    % Single-slice spin echo, one echo: a 90-degree excitation and a 180-degree
    % refocusing pulse (use='refocusing', so it is excluded from the excitation
    % count). TE = 2*(excitation->refocus spacing); a single excitation => the
    % whole sequence is one TR period.
    fov = 200e-3; Nx = 32; Ny = 32; thk = 5e-3; TE = 20e-3; TR = 500e-3;
    seq = mr.Sequence(sys);
    [rf90, gz, gzr] = mr.makeSincPulse(pi/2, sys, 'Duration', 2e-3, ...
        'SliceThickness', thk, 'apodization', 0.5, 'timeBwProduct', 4, 'use', 'excitation');
    rf180 = mr.makeSincPulse(pi, sys, 'Duration', 3e-3, ...
        'SliceThickness', thk, 'apodization', 0.5, 'timeBwProduct', 4, 'use', 'refocusing');
    deltak = 1/fov;
    gx = mr.makeTrapezoid('x', sys, 'FlatArea', Nx*deltak, 'FlatTime', 3.2e-3);
    adc = mr.makeAdc(Nx, sys, 'Duration', gx.flatTime, 'Delay', gx.riseTime);
    gxPre = mr.makeTrapezoid('x', sys, 'Area', gx.area/2, 'Duration', 1e-3); % +area: negated by the 180
    phaseAreas = ((0:Ny-1)-Ny/2)*deltak;
    grt = seq.gradRasterTime;
    % excitation-centre -> refocus-centre and refocus-centre -> echo-centre both = TE/2.
    d1 = TE/2 - (mr.calcDuration(gz) - (rf90.delay+mr.calcRfCenter(rf90))) - mr.calcDuration(gxPre) - mr.calcDuration(rf180)/2;
    d1 = floor(d1/grt)*grt;
    d2 = TE/2 - mr.calcDuration(rf180)/2 - mr.calcDuration(gx)/2;
    d2 = floor(d2/grt)*grt;
    assert(d1 >= 0 && d2 >= 0, 'negative SE delay');
    for i = 1:Ny
        seq.addBlock(rf90, gz);
        gyPre = mr.makeTrapezoid('y', sys, 'Area', phaseAreas(i), 'Duration', mr.calcDuration(gxPre));
        seq.addBlock(gxPre, gyPre, gzr);
        seq.addBlock(mr.makeDelay(d1));
        seq.addBlock(rf180);
        seq.addBlock(mr.makeDelay(d2));
        seq.addBlock(gx, adc);
        used = mr.calcDuration(rf90,gz) + mr.calcDuration(gxPre) + d1 + mr.calcDuration(rf180) + d2 + mr.calcDuration(gx);
        delayTR = floor((TR - used)/grt)*grt;
        if delayTR < 0; delayTR = 0; end
        seq.addBlock(mr.makeDelay(delayTR));
    end
    seq.setDefinition('FOV', [fov fov thk]);
    p = struct('family','se2d', 'flip_deg',90, 'n_slices',1, ...
        'tr_s',TR, 'te_s',TE, 'echo_spacing_s',NaN);
end
