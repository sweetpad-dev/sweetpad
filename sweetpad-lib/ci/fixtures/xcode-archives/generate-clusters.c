// Generate an archive with Apple's native DAT deduplication implementation.
#include <AppleArchive/AppleArchive.h>
#include <assert.h>
#include <fcntl.h>

int main(int argc, char **argv) {
  assert(argc == 4);
  AAFlagSet flags = argv[3][0] == '1' ? AA_FLAG_ARCHIVE_DEDUPLICATE_DAT : 0;
  AAPathList paths = AAPathListCreateWithDirectoryContents(argv[1], NULL, NULL, NULL, 0, 1);
  AAFieldKeySet fields = AAFieldKeySetCreateWithString("TYP,PAT,LNK,MOD,DAT,SIZ,SH2,HLC,CLC,SLC");
  AAByteStream file = AAFileStreamOpenWithPath(argv[2], O_WRONLY | O_CREAT | O_EXCL, 0600);
  assert(paths && fields && file);
  AAArchiveStream archive = AAEncodeArchiveOutputStreamOpen(file, NULL, NULL, flags, 1);
  assert(archive);
  int result = AAArchiveStreamWritePathList(archive, paths, fields, argv[1], NULL, NULL, flags, 1);
  int closed = AAArchiveStreamClose(archive);
  int file_closed = AAByteStreamClose(file);
  AAFieldKeySetDestroy(fields);
  AAPathListDestroy(paths);
  return result < 0 || closed < 0 || file_closed < 0;
}
