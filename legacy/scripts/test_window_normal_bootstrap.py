"""Guarded rejection/order controls for actual normal bootstrap source.

All descriptors, metadata, source bytes and resource calls are explicit mocks.
These tests do not read actual interpreter/image/target files, authenticate a
family, load an extension, execute a child or qualify filesystem durability.
The real _read_file hashing/bounds and negative bootstrap paths are exercised.
Full positive Admission/bootstrap integration remains a separate parent gate.
"""
import hashlib
import json
import stat
import sys
import types

import test_window_normal_host as policy

class BootstrapIO(policy.Guarded):
    def details(self,raw=b'abcdef',**change):
        result=dict(st_dev=1,st_ino=41,st_uid=501,st_mode=stat.S_IFREG|0o400,
                    st_size=len(raw),st_nlink=1,st_mtime_ns=20,st_ctime_ns=30)
        result.update(change);return types.SimpleNamespace(**result)
    def reader(self,raw=b'abcdef',*,before=None,after=None,path=None,chunks=None):
        h=self.host;before=before or self.details(raw);after=after or before;path=path or after
        calls=[];states=iter((before,after));parts=list(chunks if chunks is not None else (raw,b''))
        def opened(name,flags,*,dir_fd=None):
            calls.append(('open',name,flags,dir_fd));return 55
        def read(fd,size):
            self.assertEqual(fd,55);self.assertGreater(size,0);self.assertLessEqual(size,65536)
            calls.append(('read',size));return parts.pop(0) if parts else b''
        self.patch(h.os,'open',opened);self.patch(h.os,'fstat',lambda fd:next(states))
        self.patch(h.os,'read',read);self.patch(h.os,'stat',lambda *args,**kwargs:path)
        return calls
    def test_reader_hashes_actual_bytes_and_forwards_nofollow_dirfd(self):
        raw=b'abcdef';calls=self.reader(raw,chunks=(b'abc',b'def',b''))
        pin=dict(bytes=6,mode=0o400,sha256=hashlib.sha256(raw).hexdigest())
        self.assertEqual(self.host._read_file('source.py',64,501,root_fd=7,pin=pin),raw)
        self.assertEqual(calls[0][1::2],('source.py',7))
        self.assertTrue(calls[0][2]&self.host.os.O_NOFOLLOW)
        self.assertTrue(calls[0][2]&self.host.os.O_CLOEXEC)
        self.assertIn(55,self.host._LIFETIME)
    def test_reader_rejects_changed_path_identity(self):
        self.reader(path=self.details(st_ino=42))
        with self.assertRaisesRegex(ValueError,'changed'):self.host._read_file('source.py',64,501)
        self.assertIn(55,self.host._LIFETIME)
    def test_reader_rejects_changed_descriptor_metadata(self):
        self.reader(after=self.details(st_mtime_ns=21))
        with self.assertRaisesRegex(ValueError,'changed'):self.host._read_file('source.py',64,501)
    def test_reader_rejects_growth_beyond_original_size(self):
        self.reader(chunks=(b'abcdefg',))
        with self.assertRaisesRegex(ValueError,'changed'):self.host._read_file('source.py',64,501)
    def test_reader_rejects_truncated_bytes(self):
        self.reader(chunks=(b'abc',b''))
        with self.assertRaisesRegex(ValueError,'changed'):self.host._read_file('source.py',64,501)
    def test_reader_rejects_wrong_digest(self):
        self.reader()
        with self.assertRaisesRegex(ValueError,'digest'):
            self.host._read_file('source.py',64,501,pin=dict(bytes=6,mode=0o400,sha256='0'*64))
    def test_reader_rejects_hardlinks_symlinks_writable_or_foreign_owner(self):
        for values in (dict(st_nlink=2),dict(st_mode=stat.S_IFLNK|0o400),
                       dict(st_mode=stat.S_IFREG|0o422),dict(st_uid=7)):
            self.reader(before=self.details(**values))
            with self.assertRaisesRegex(ValueError,'Owned'):self.host._read_file('source.py',64,501)
    def test_reader_rejects_boolean_pin_size(self):
        self.reader(b'x')
        with self.assertRaisesRegex(ValueError,'digest'):
            self.host._read_file('source.py',64,501,pin=dict(bytes=True,mode=0o400,sha256=hashlib.sha256(b'x').hexdigest()))
    def test_fd_is_retained_before_fstat_failure(self):
        self.patch(self.host.os,'open',lambda *args,**kwargs:55)
        def failed(fd):raise OSError('synthetic descriptor read loss')
        self.patch(self.host.os,'fstat',failed)
        with self.assertRaises(OSError):self.host._read_file('source.py',64,501)
        self.assertIn(55,self.host._LIFETIME)

class BootstrapRefusal(policy.Guarded):
    def family(self):return '/private/tmp/sims-normal-true-'+'d'*32
    def fixture(self,*,change=None,raw=None,siblings=None,python=b'wrong interpreter'):
        h=self.host;family=self.family();calls=[]
        identity=dict(device=1,inode=41,uid=501,mode=0o700)
        root=types.SimpleNamespace(st_dev=1,st_ino=41,st_uid=501,st_mode=stat.S_IFDIR|0o700)
        files={n:dict(bytes=16,mode=0o400,sha256='1'*64) for n in h.FILES}
        files['_window_owned_child_bridge.so']=dict(bytes=109552,mode=0o400,
            sha256='2b591afbf653005501243afcc848f94d5e38b95e7323c2fb9826595c6cf27bf6')
        manifest=dict(schema=1,profile=h.PROFILE,root=identity,fifo={},token='a'*32,files=files,
                      child_cwd={},target={},python=dict(path=h.PYTHON,sha256=h.PYTHON_SHA256))
        if change:change(manifest)
        raw=raw if raw is not None else json.dumps(manifest).encode()
        pin=hashlib.sha256(raw).hexdigest()
        self.patch(h.os,'lstat',lambda *args:root);self.patch(h.os,'fstat',lambda *args:root)
        self.patch(h.os,'open',lambda *args,**kwargs:55)
        self.patch(h.os,'listdir',lambda *args:siblings if siblings is not None else list(h.FILES)+['manifest.json','control.fifo','child-cwd'])
        def read(name,*args,**kwargs):
            calls.append(name)
            if name=='manifest.json':return raw
            if name==h.PYTHON:return python
            return b'explicit synthetic source bytes; not authentication'
        self.patch(h,'_read_file',read)
        return family,pin,calls
    def test_wrong_family_refused_before_os_calls(self):
        with self.assertRaises(ValueError):self.host._bootstrap_sources('/private/tmp/sims-suspended-true-'+'d'*32,'a'*64,501)
    def test_nonprivate_root_refused_before_open(self):
        root=types.SimpleNamespace(st_mode=stat.S_IFDIR|0o777,st_uid=501)
        self.patch(self.host.os,'lstat',lambda *args:root)
        with self.assertRaises(ValueError):self.host._bootstrap_sources(self.family(),'a'*64,501)
    def test_extra_initial_sibling_refused_before_any_source_read(self):
        family,pin,calls=self.fixture(siblings=list(self.host.FILES)+['manifest.json','control.fifo','child-cwd','unexpected'])
        with self.assertRaises(ValueError):self.host._bootstrap_sources(family,pin,501)
        self.assertEqual(calls,[])
    def test_wrong_manifest_digest_refused_before_decode(self):
        family,pin,calls=self.fixture(raw=b'not JSON')
        with self.assertRaisesRegex(ValueError,'digest'):self.host._bootstrap_sources(family,'0'*64,501)
        self.assertEqual(calls,['manifest.json'])
    def test_duplicate_keys_refused_before_source_reads(self):
        family,pin,calls=self.fixture(raw=b'{"schema":1,"schema":1}')
        with self.assertRaisesRegex(ValueError,'Duplicate'):self.host._bootstrap_sources(family,pin,501)
        self.assertEqual(calls,['manifest.json'])
    def test_nonfinite_json_refused_before_source_reads(self):
        family,pin,calls=self.fixture(raw=b'{"schema":NaN}')
        with self.assertRaisesRegex(ValueError,'Finite'):self.host._bootstrap_sources(family,pin,501)
    def test_wrong_profile_or_boolean_schema_refused(self):
        for values in (dict(profile='SUSPENDED_TRUE_ONLY_NO_RESUME_NO_PUBLICATION'),dict(schema=True)):
            family,pin,calls=self.fixture(change=lambda m,values=values:m.update(values))
            with self.assertRaises(ValueError):self.host._bootstrap_sources(family,pin,501)
            self.assertEqual(calls,['manifest.json'])
    def test_missing_source_refused_before_source_reads(self):
        family,pin,calls=self.fixture(change=lambda m:m['files'].pop('window_normal_diagnostic.py'))
        with self.assertRaises(ValueError):self.host._bootstrap_sources(family,pin,501)
        self.assertEqual(calls,['manifest.json'])
    def test_executable_or_unknown_source_pin_mode_refused(self):
        for mode in (0o755,True):
            family,pin,calls=self.fixture(change=lambda m,mode=mode:m['files']['window_normal_host.py'].update(mode=mode))
            with self.assertRaises(ValueError):self.host._bootstrap_sources(family,pin,501)
            self.assertEqual(calls,['manifest.json'])
    def test_wrong_image_pin_refused_before_interpreter_read(self):
        family,pin,calls=self.fixture(change=lambda m:m['files']['_window_owned_child_bridge.so'].update(sha256='0'*64))
        with self.assertRaisesRegex(ValueError,'image pin'):self.host._bootstrap_sources(family,pin,501)
        self.assertNotIn(self.host.PYTHON,calls)
    def test_wrong_interpreter_digest_refused_without_module_execution(self):
        family,pin,calls=self.fixture()
        with self.assertRaisesRegex(ValueError,'interpreter'):self.host._bootstrap_sources(family,pin,501)
        self.assertEqual(calls[-1],self.host.PYTHON)
    def test_failed_source_execution_keeps_original_partial_module(self):
        name='_synthetic_partial_normal_source'
        self.addCleanup(sys.modules.pop,name,None)
        with self.assertRaisesRegex(ValueError,'synthetic'):
            self.host._source(name,b'raise ValueError("synthetic source failure")','/private/tmp/synthetic.py')
        module=sys.modules[name]
        self.assertIn(module,self.host._LIFETIME)

class BootstrapEnvironment(policy.Guarded):
    def test_only_exact_minimal_environment_is_accepted(self):
        valid=dict(PATH='/usr/bin:/bin',LC_ALL='C',TMPDIR='/private/tmp',SIMS_NORMAL_MANIFEST_SHA256='e'*64)
        self.assertEqual(self.host._environment(valid),'e'*64)
        self.assertEqual(self.host._environment(dict(valid,__CF_USER_TEXT_ENCODING='0x1:0:0')),'e'*64)
        for change in (dict(DYLD_LIBRARY_PATH='/tmp'),dict(PATH='/tmp'),
                       dict(SIMS_NORMAL_MANIFEST_SHA256='e'*63),dict(__CF_USER_TEXT_ENCODING='x'*129)):
            with self.assertRaises(ValueError):self.host._environment(dict(valid,**change))
    def test_resource_policy_caps_portable_infinity_and_avoids_sigchld(self):
        calls=[]
        self.patch(policy.guards.resource,'getrlimit',lambda key:(policy.guards.resource.RLIM_INFINITY,)*2)
        self.patch(policy.guards.resource,'setrlimit',lambda key,values:calls.append(('limit',key,values)))
        self.patch(policy.guards.signal,'signal',lambda number,callback:calls.append(('handler',number,callback)))
        self.host._policies()
        self.assertEqual(calls[:2],[('limit',policy.guards.resource.RLIMIT_CORE,(0,0)),
                                   ('limit',policy.guards.resource.RLIMIT_FSIZE,(64*1024**2,)*2)])
        handlers=[item[1] for item in calls if item[0]=='handler']
        self.assertEqual(set(handlers),{policy.guards.signal.SIGINT,policy.guards.signal.SIGTERM,policy.guards.signal.SIGHUP})
        self.assertNotIn(policy.guards.signal.SIGCHLD,handlers)
