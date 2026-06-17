function ipp = fnint(pp)
%FNINT Antiderivative of a 1-D piecewise polynomial — a base-MATLAB shim for the
%   Curve Fitting Toolbox `fnint`, which `mr.Sequence.testReport` /
%   `calculateKspacePP` need to integrate gradient waveforms into k-space.
%
%   The Pulseq mr-toolbox already has an Octave path (`ppint`) for exactly this;
%   on a MATLAB install without the Curve Fitting Toolbox neither `fnint` nor
%   `ppint` exists, so placing this drop-in on the path lets the corpus generator
%   run `testReport()` as the independent oracle. It integrates each polynomial
%   piece in its local variable and accumulates a running offset so the result is
%   continuous and zero at the first break — the defining property of `fnint`.
%
%   Validated against the known generation parameters: for the corpus GRE the
%   resulting testReport TE/TR/duration reproduce the inputs exactly.
[breaks, coefs, L, K, D] = unmkpp(pp);
if D ~= 1
    error('fnint shim supports 1-D piecewise polynomials only (D=%d)', D);
end
icoefs = zeros(L, K + 1);
offset = 0;
for i = 1:L
    p = coefs(i, :);
    ip = [p ./ (K:-1:1), offset];   % antiderivative; constant term = value at left break
    icoefs(i, :) = ip;
    offset = polyval(ip, breaks(i + 1) - breaks(i));   % carry continuity to next piece
end
ipp = mkpp(breaks, icoefs, 1);
end
