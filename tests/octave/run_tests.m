function run_tests
  assert(1 + 1 == 2);

  repo_root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
  input_file = fullfile(repo_root, 'tests', 'fixtures', 'formatter', 'runtime', 'input.m');
  expected_file = fullfile(repo_root, 'tests', 'fixtures', 'formatter', 'runtime', 'expected.m');

  run(input_file);
  clear x y;
  run(expected_file);

  disp('mstyle Octave formatter regression tests passed');
end
